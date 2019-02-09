#![allow(dead_code)]
use crate::arch;
use crate::bio;
use crate::file::{self, File};
use crate::fslog;
use crate::param;
use crate::proc;
use crate::sleeplock::Sleeplock;
use crate::spinlock::SpinMutex as Mutex;
use crate::Result;
use core::cell::{Cell, RefCell};
use core::cmp;
use core::intrinsics::{volatile_copy_memory, volatile_set_memory};
use core::mem;
use core::slice;
use seq_macro::seq;
use static_assertions::const_assert_eq;
use syslib::stat::{FileType, Stat};

// On-disk file system format.
// Both the kernel and user programs use this header file.

pub const BSIZE: usize = arch::PAGE_SIZE;

const ROOTINO: u64 = 1;

// Disk layout:
// [ boot block | super block | log | inode blocks |
//                                          free bit map | data blocks]
//
// mkfs computes the super block and builds an initial file system. The
// super block describes the disk layout:
#[repr(C)]
#[derive(Debug)]
pub struct Superblock {
    size: u64,        // Size of file system image in blocks
    nblocks: u64,     // Number of data blocks
    ninodes: u64,     // Number of inodes.
    pub nlog: u64,        // Number of log blocks
    pub log_start: u64,   // Block number of first log block
    inode_start: u64, // Block number of first inode block
    bmap_start: u64,  // Block number of first free map block
}

impl Superblock {
    pub const fn iblock(&self, inum: u64) -> u64 {
        inum / IPB as u64 + self.inode_start
    }

    // Block of free map containing bit for block b
    pub const fn bblock(&self, b: u64) -> u64 {
        b / BPB as u64 + self.bmap_start
    }

    pub const fn new() -> Superblock {
        Superblock {
            size: 0,
            nblocks: 0,
            ninodes: 0,
            nlog: 0,
            log_start: 0,
            inode_start: 0,
            bmap_start: 0,
        }
    }

    pub fn read(dev: u32) -> Result<Superblock> {
        bio::with_block(dev, ROOTINO, |bp| {
            let mut sb = Self::new();
            unsafe {
                volatile_copy_memory(&mut sb, bp.data() as *const Superblock, 1);
            }
            sb
        })
    }
}

static mut SUPERBLOCK: Superblock = Superblock::new();

const NDIRECT: usize = 12;
const NINDIRECT: usize = BSIZE / mem::size_of::<u64>();
const MAXFILE: usize = NDIRECT + NINDIRECT;

// On-disk inode structure
#[derive(Debug)]
#[repr(C)]
struct DInode {
    typ: u32,                  // File type
    major: u32,                // Major device number (T_DEV only)
    minor: u32,                // Minor device number (T_DEV only)
    nlink: u32,                // Number of links to inode in file system
    size: u64,                 // Size of file (bytes)
    addrs: [u64; NDIRECT + 1], // Data block addresses
}
const_assert_eq!(mem::size_of::<DInode>(), 128);

impl DInode {
    pub const fn new() -> DInode {
        DInode {
            typ: 0,
            major: 0,
            minor: 0,
            nlink: 0,
            size: 0,
            addrs: [0; NDIRECT + 1],
        }
    }
}

// Inodes per block.
const IPB: usize = BSIZE / mem::size_of::<DInode>();

// Bitmap bits per block
const BPB: usize = BSIZE * 8;

// Directory is a file containing a sequence of dirent structures.
const DIRSIZ: usize = 24;

#[derive(Default)]
struct Dirent {
    inum: u64,
    name: [u8; DIRSIZ],
}
const_assert_eq!(mem::size_of::<Dirent>(), 32);

// Zero a block.
fn bzero(dev: u32, blockno: u64) {
    let bp = bio::read(dev, blockno).expect("block read");
    unsafe {
        volatile_set_memory(bp.data_mut().as_mut_ptr(), 0, BSIZE);
    }
    fslog::write(bp);
    bp.relse();
}

// Allocate a zeroed storage block.
fn balloc(dev: u32, sb: &Superblock) -> Result<u64> {
    for b in (0..sb.size).step_by(BPB) {
        let bp = bio::read(dev, sb.bblock(b))?;
        for bi in 0..BPB as u64 {
            if b + bi >= sb.size {
                break;
            }
            let data = bp.data_mut();
            let m = 1 << (bi % 8);
            let k = (bi / 8) as usize;
            if data[k] & m == 0 {
                data[k] |= m;
                fslog::write(bp);
                bp.relse();
                bzero(dev, b + bi);
                return Ok(b + bi);
            }
        }
        bp.relse();
    }
    Err("balloc: out of blocks")
}

// Free a storage block.
fn bfree(dev: u32, blockno: u64, sb: &Superblock) {
    bio::with_block(dev, sb.bblock(blockno), |bp| {
        let bi = blockno as usize % BPB;
        let m = 1 << (bi % 8);
        let data = bp.data_mut();
        assert_eq!(m, data[bi / 8] & m, "freeing free block");
        data[bi / 8] &= !m;
        fslog::write(bp);
    })
    .expect("bfree");
}

// Inodes.
//
// An inode describes a single unnamed file.
// The inode disk structure holds metadata: the file's type,
// its size, the number of links referring to it, and the
// list of blocks holding the file's content.
//
// The inodes are laid out sequentially on disk at
// sb.startinode. Each inode has a number, indicating its
// position on the disk.
//
// The kernel keeps a cache of in-use inodes in memory
// to provide a place for synchronizing access
// to inodes used by multiple processes. The cached
// inodes include book-keeping information that is
// not stored on disk: ip->ref and ip->valid.
//
// An inode and its in-memory representation go through a
// sequence of states before they can be used by the
// rest of the file system code.
//
// * Allocation: an inode is allocated if its type (on disk)
//   is non-zero. ialloc() allocates, and put() frees if
//   the reference and link counts have fallen to zero.
//
// * Referencing in cache: an entry in the inode cache
//   is free if ip->ref is zero. Otherwise ip->ref tracks
//   the number of in-memory pointers to the entry (open
//   files and current directories). Inode::get() finds or
//   creates a cache entry and increments its ref; put()
//   decrements ref.
//
// * Valid: the information (type, size, &c) in an inode
//   cache entry is only correct when ip->valid is 1.
//   lock() reads the inode from the disk and sets
//   ip->valid, while put() clears ip->valid if ip->ref has fallen to zero.
//
// * Locked: file system code may only examine and modify
//   the information in an inode and its content if it
//   has first locked the inode.
//
// Thus a typical sequence is:
//   ip = Inode::get(dev, inum)
//   ip.lock(ip)
//   ... examine and modify ip->xxx ...
//   ip.unlock(ip)
//   ip.put(ip)
//
// lock() is separate from get() so that system calls can
// get a long-term reference to an inode (as for an open file)
// and only lock it for short periods (e.g., in read()).
// The separation also helps avoid deadlock and races during
// pathname lookup. get() increments ip->ref so that the inode
// stays cached and pointers to it remain valid.
//
// Many internal file system functions expect the caller to
// have locked the inodes involved; this lets callers create
// multi-step atomic operations.
//
// The icache.lock spin-lock protects the allocation of icache
// entries. Since ip->ref indicates whether an entry is free,
// and ip->dev and ip->inum indicate which i-node an entry
// holds, one must hold icache.lock while using any of those fields.
//
// An ip->lock sleep-lock protects all ip-> fields other than ref,
// dev, and inum.  One must hold ip->lock in order to
// read or write that inode's ip->valid, ip->size, ip->type, &c.

// XXX(cross): Remove this assert after Rust RFC 2203 is implemented.
// For the time being, we have to keep the manifest in the
// seq!() invocation in sync with NINODE.
static ICACHE: Mutex<[Inode; param::NINODE]> =
    Mutex::new("icache", seq!(N in 0..1024 { [#(Inode::new(),)*] }));
const_assert_eq!(param::NINODE, 1024);

pub unsafe fn init(dev: u32) {
    SUPERBLOCK = Superblock::read(dev).expect("superblock read failed");
}

pub unsafe fn superblock() -> &'static Superblock {
    &SUPERBLOCK
}

#[allow(clippy::mut_from_ref)]
unsafe fn buf_to_dinode(bp: &bio::Buf, inum: usize) -> &mut DInode {
    &mut *(bp.data() as *mut DInode).add(inum % IPB)
}

// Allocate an inode on device dev.
// Mark it as allocated by  giving it type type.
// Returns an unlocked but allocated and referenced inode.
pub fn ialloc(dev: u32, typ: FileType, sb: &'static Superblock) -> Result<&'static Inode> {
    for inum in 1..sb.ninodes {
        let bp = bio::read(dev, sb.iblock(inum))?;
        let di = unsafe { buf_to_dinode(bp, inum as usize) };
        if di.typ == 0 {
            di.typ = typ as u32;
            fslog::write(bp);
            bp.relse();
            return Inode::get(dev, inum, sb);
        }
        bp.relse();
    }
    Err("ialloc: no inodes")
}

#[derive(Debug)]
pub struct InodeMeta {
    dev: u32,
    inum: u64,
    sb: Option<&'static Superblock>,
}

impl InodeMeta {
    pub const fn empty() -> InodeMeta {
        InodeMeta {
            dev: 0,
            inum: 0,
            sb: None,
        }
    }

    pub const fn new(dev: u32, inum: u64, superblock: &'static Superblock) -> InodeMeta {
        InodeMeta {
            dev,
            inum,
            sb: Some(superblock),
        }
    }
}

// In-memory representation of an inode.
#[derive(Debug)]
pub struct Inode {
    meta: RefCell<InodeMeta>, // Device number
    ref_cnt: Cell<u32>,       // Reference count

    lock: Sleeplock,   // Protects everything below here
    valid: Cell<bool>, // Has inode been read from disk?

    dinode: RefCell<DInode>, // disk inode data.
}

impl Inode {
    pub const fn new() -> Inode {
        Inode {
            meta: RefCell::new(InodeMeta::empty()),
            ref_cnt: Cell::new(0),
            lock: Sleeplock::new("inode"),
            valid: Cell::new(false),
            dinode: RefCell::new(DInode::new()),
        }
    }

    fn dev(&self) -> u32 {
        self.meta.borrow().dev
    }

    fn inum(&self) -> u64 {
        self.meta.borrow().inum
    }

    fn ref_cnt(&self) -> u32 {
        self.ref_cnt.get()
    }

    fn inc_ref_cnt(&self) {
        self.ref_cnt.set(self.ref_cnt() + 1);
    }

    fn dec_ref_cnt(&self) {
        self.ref_cnt.set(self.ref_cnt() - 1);
    }

    fn is_valid(&self) -> bool {
        self.valid.get()
    }

    fn typ(&self) -> FileType {
        let typ = self.dinode.borrow().typ;
        match typ {
            0 => FileType::Unused,
            1 => FileType::Dir,
            2 => FileType::File,
            3 => FileType::Dev,
            _ => panic!("bad inode file type: {}", typ),
        }
    }

    fn nlink(&self) -> u32 {
        self.dinode.borrow().nlink
    }

    fn size(&self) -> u64 {
        self.dinode.borrow().size
    }

    fn set_size(&self, size: u64) {
        self.dinode.borrow_mut().size = size;
    }

    // Copy a modified in-memory inode to the log.
    // Must be called after every change to an ip->xxx field
    // that lives on the storage device, since the inode cache
    // is write-through.  The Caller must hold the inode's lock.
    fn update(&self) -> Result<()> {
        let sb = self.meta.borrow().sb.expect("update requires superblock");
        bio::with_block(self.dev(), sb.iblock(self.inum()), |bp| {
            let di = unsafe { buf_to_dinode(bp, self.inum() as usize) };
            let dip = di as *mut DInode;
            unsafe {
                volatile_copy_memory(dip, self.dinode.as_ptr(), 1);
            }
            fslog::write(bp);
        })
    }

    // Increments the reference count for this inode.
    // Returns `self` to enable `ip = ip1.dup()` idiom.
    pub fn dup(&self) -> &Inode {
        ICACHE.with_lock(|_| {
            self.inc_ref_cnt();
        });
        self
    }

    // Lock the given inode, reading it from the storage device
    // if necessary.
    pub fn lock(&self) {
        assert!(self.ref_cnt() > 0, "ilock unref inode");
        let sb = self.meta.borrow().sb.expect("update requires superblock");
        self.lock.acquire();
        if !self.is_valid() {
            bio::with_block(self.dev(), sb.iblock(self.inum()), |bp| {
                let di = unsafe { buf_to_dinode(bp, self.inum() as usize) };
                let dip = di as *const DInode;
                unsafe {
                    volatile_copy_memory(self.dinode.as_ptr(), dip, 1);
                }
            })
            .expect("block read");
            self.valid.set(true);
            assert_ne!(self.typ(), FileType::Unused, "ilock: no type");
        }
    }

    // Unlocks this inode.
    pub fn unlock(&self) {
        assert!(self.lock.holding() && self.ref_cnt() > 0, "inode unlock");
        self.lock.release();
    }

    // Find or allocate an icache entry for the inode with number
    // `inum` on device `dev and and return the in-memory copy.
    // Does not lock the inode and does not read it from disk.
    fn get(dev: u32, inum: u64, sb: &'static Superblock) -> Result<&'static Inode> {
        let icache = ICACHE.lock();
        let mut empty = None;
        for ip in icache.iter() {
            let ip = unsafe { &*(ip as *const Inode) };
            if ip.ref_cnt() > 0 && ip.dev() == dev && ip.inum() == inum {
                ip.inc_ref_cnt();
                return Ok(ip);
            }
            if empty.is_none() && ip.ref_cnt() == 0 {
                empty = Some(ip);
            }
        }
        if empty.is_none() {
            return Err("Inode::get: no inodes");
        }
        let ip = empty.unwrap();
        let mut meta = ip.meta.borrow_mut();
        meta.dev = dev;
        meta.inum = inum;
        meta.sb = Some(sb);
        ip.inc_ref_cnt();
        ip.valid.set(true);
        Ok(ip)
    }

    // Drop a reference to an in-memory inode.
    // If that was the last reference, the inode cache entry can
    // be recycled.
    // If that was the last reference and the inode has no links
    // to it, free the inode (and its content) on disk.
    // All calls to put() must be inside a transaction in
    // case it has to free the inode.
    pub fn put(&self) -> Result<()> {
        self.lock.acquire();
        if self.is_valid() && self.nlink() == 0 {
            let ref_cnt = ICACHE.with_lock(|_| self.ref_cnt());
            if ref_cnt == 1 {
                // inode has no links or other references.
                // Truncate and free.
                self.trunc()?;
                self.dinode.borrow_mut().typ = 0;
                self.update()?;
                self.valid.set(false);
            }
        }
        self.lock.release();

        ICACHE.with_lock(|_| self.dec_ref_cnt());
        Ok(())
    }

    pub fn unlock_put(&self) -> Result<()> {
        self.unlock();
        self.put()
    }

    // Return the disk block address of the nth block in the
    // inode.  If there is no such block then allocate one.
    fn bmap(&self, bn: u64) -> Result<u64> {
        assert!(self.lock.holding(), "bmap on unlocked inode");
        let sb = self.meta.borrow().sb.expect("bmap requires superblock");
        let addrs = &mut self.dinode.borrow_mut().addrs;
        let bn = bn as usize;
        if bn < NDIRECT {
            if addrs[bn] == 0 {
                addrs[bn] = balloc(self.dev(), sb)?;
            }
            return Ok(addrs[bn]);
        }
        let bn = bn - NDIRECT;
        if bn < NINDIRECT {
            if addrs[NDIRECT] == 0 {
                addrs[NDIRECT] = balloc(self.dev(), sb)?;
            }
            let addr = addrs[NDIRECT];
            return bio::with_block(self.dev(), addr, |bp| {
                let iaddrs = unsafe { slice::from_raw_parts_mut(bp.data() as *mut u64, NINDIRECT) };
                if iaddrs[bn] == 0 {
                    iaddrs[bn] = balloc(self.dev(), sb)?;
                    fslog::write(bp);
                }
                Ok(iaddrs[bn])
            })?;
        }
        Err("bmap: out of range")
    }

    fn trunc(&self) -> Result<()> {
        assert!(self.lock.holding(), "truncating unlocked inode");
        let addrs = &mut self.dinode.borrow_mut().addrs;
        let sb = &self
            .meta
            .borrow()
            .sb
            .expect("allocated inode sans superblock ref");
        for addr in addrs.iter_mut().take(NDIRECT).filter(|addr| **addr != 0) {
            bfree(self.dev(), *addr, sb);
            *addr = 0;
        }
        if addrs[NDIRECT] != 0 {
            bio::with_block(self.dev(), addrs[NDIRECT], |bp| {
                let addrs = unsafe { &mut *(bp.data() as *mut [u64; NINDIRECT]) };
                for addr in addrs.iter_mut().filter(|addr| **addr != 0) {
                    bfree(self.dev(), *addr, sb);
                    *addr = 0;
                }
            })?;
            bfree(self.dev(), addrs[NDIRECT], sb);
        }
        self.dinode.borrow_mut().size = 0;
        self.update()
    }

    fn stati(&self) -> Stat {
        Stat {
            typ: self.typ(),
            dev: self.dev(),
            ino: self.inum(),
            nlink: self.nlink(),
            size: self.size(),
        }
    }

    pub fn readi<T>(&self, dst: &mut [T], off: u64) -> Result<usize> {
        if off > self.size() {
            return Err("offset beyond end of file");
        }
        let dstlen = mem::size_of::<T>() * dst.len();
        if off.wrapping_add(dstlen as u64) < off {
            return Err("offset and length wrap");
        }
        let mut off = off as usize;
        let dstptr = dst.as_mut_ptr() as *mut u8;
        let n = cmp::min(dstlen, self.size() as usize - off);
        for total in (0..n).step_by(BSIZE) {
            bio::with_block(self.dev(), self.bmap((off / BSIZE) as u64)?, |bp| {
                let m = cmp::min(n - total, BSIZE - off % BSIZE);
                unsafe {
                    volatile_copy_memory(dstptr.add(total), bp.data_ref().as_ptr(), m);
                }
                off += m;
            })?;
        }
        Ok(n as usize)
    }

    fn writei(&self, src: &[u8], off: u64) -> Result<usize> {
        if off > self.size() {
            return Err("offset beyond end of file");
        }
        if off.wrapping_add(src.len() as u64) < off {
            return Err("offset and length wrap");
        }
        if off as usize + src.len() > MAXFILE * BSIZE {
            return Err("write makes file too big");
        }
        let mut off = off as usize;
        let n = cmp::min(src.len(), self.size() as usize - off);
        for total in (0..n).step_by(BSIZE) {
            bio::with_block(self.dev(), self.bmap((off / BSIZE) as u64)?, |bp| {
                let m = cmp::min(n - total, BSIZE - off % BSIZE);
                unsafe {
                    volatile_copy_memory(
                        bp.data_mut().as_mut_ptr().add(total + off % BSIZE),
                        src.as_ptr().add(total),
                        m,
                    );
                }
                fslog::write(bp);
                off += m;
            })?;
        }
        if n > 0 && off > self.size() as usize {
            self.set_size(off as u64);
            self.update()?;
        }
        Ok(n)
    }

    // Directories.
    //
    // Directories are just files, but they have additional special semantics.
    pub fn dir_lookup_offset(&self, name: &[u8]) -> Result<(&'static Inode, u64)> {
        assert_eq!(self.typ(), FileType::Dir, "dir_lookup not in a directory");
        let de_size = mem::size_of::<Dirent>();
        for off in (0..self.size()).step_by(de_size) {
            let mut entry = Dirent::default();
            let entry_slice = {
                let ptr = &mut entry as *mut Dirent as *mut u8;
                unsafe { slice::from_raw_parts_mut(ptr, de_size) }
            };
            let nread = self.readi(entry_slice, off)?;
            assert_eq!(nread, de_size, "dir_lookup read");
            if entry.inum == 0 {
                continue;
            }
            if entry.name == name {
                let sb = self.meta.borrow().sb.expect("superblockless inode");
                let ip = Self::get(self.dev(), entry.inum, sb)?;
                return Ok((ip, off));
            }
        }
        Err("file not found")
    }

    pub fn dir_lookup(&self, name: &[u8]) -> Result<&'static Inode> {
        let (ip, _) = self.dir_lookup_offset(name)?;
        Ok(ip)
    }

    pub fn dir_link(&self, name: &[u8], inum: u64) -> Result<()> {
        if let Ok(ip) = self.dir_lookup(name) {
            ip.put()?;
            return Err("file already exists");
        }
        let de_size = mem::size_of::<Dirent>();
        let mut off = 0;
        let mut entry = Dirent::default();
        let entry_slice = {
            let ptr = &mut entry as *mut Dirent as *mut u8;
            unsafe { slice::from_raw_parts_mut(ptr, de_size) }
        };
        for o in (0..self.size()).step_by(de_size) {
            off = o;
            let nread = self.readi(entry_slice, off)?;
            assert_eq!(nread, de_size, "dir_lookup read");
            if entry.inum == 0 {
                break;
            }
        }
        let len = cmp::min(DIRSIZ, name.len());
        unsafe {
            volatile_set_memory(entry_slice.as_mut_ptr(), 0, de_size);
            volatile_copy_memory(entry.name.as_mut_ptr(), name.as_ptr(), len);
        }
        entry.inum = inum;
        self.writei(entry_slice, off)?;
        Ok(())
    }

    pub fn with_putlock<U, F: FnMut(&Inode) -> Result<U>>(&self, mut thunk: F) -> Result<U> {
        self.lock();
        let r = thunk(self);
        self.unlock_put()?;
        r
    }

    pub fn with_lock<U, F: FnMut(&Inode) -> U>(&self, mut thunk: F) -> U {
        self.lock();
        let r = thunk(self);
        self.unlock();
        r
    }
}

// Copy the next path element from path into name.
// Return a pointer to the element following the copied one.
// The returned path has no leading slashes,
// so the caller can check *path=='\0' to see if the name is the last one.
// If no name to remove, return 0.
//
// Examples:
//   skip_elem(b"a/bb/c") -> Some(b"a", b"bb/c")
//   skip_elem(b"///a//bb") -> Some(b"a", b"bb")
//   skip_elem(b"a") -> Some(b"a", "")
//   skip_elem(b"") = skip_elem(b"////") = None
#[allow(clippy::or_fun_call)]
fn skip_elem(path: &[u8]) -> Option<(&[u8], &[u8])> {
    let start = path.iter().position(|p| *p != b'/')?;
    let path = &path[start..];
    let end = path.iter().position(|b| *b == b'/').unwrap_or(path.len());
    let name = &path[..cmp::min(end, DIRSIZ)];
    let path = &path[end..];
    let next_start = path.iter().position(|b| *b != b'/').unwrap_or(path.len());
    let path = &path[next_start..];
    Some((name, path))
}

#[cfg(test)]
mod skip_elem_tests {

    #[test]
    fn test_works() {
        use super::skip_elem;
        assert_eq!(skip_elem(&b"a/bb/c"[..]), Some((&b"a"[..], &b"bb/c"[..])));
        assert_eq!(skip_elem(&b"///a//bb"[..]), Some((&b"a"[..], &b"bb"[..])));
        assert_eq!(skip_elem(&b"///a//"[..]), Some((&b"a"[..], &b""[..])));
        assert_eq!(skip_elem(&b"///aa//bb"[..]), Some((&b"aa"[..], &b"bb"[..])));
        assert_eq!(skip_elem(&b"///aa//b"[..]), Some((&b"aa"[..], &b"b"[..])));
        assert_eq!(skip_elem(&b"a"[..]), Some((&b"a"[..], &b""[..])));
        assert_eq!(skip_elem(&b""[..]), None);
        assert_eq!(skip_elem(&b"////"[..]), None);
    }
}

fn is_dir(ip: &Inode) -> Result<&Inode> {
    if ip.typ() != FileType::Dir {
        return Err("not a directory");
    }
    Ok(ip)
}

pub fn namex<F>(mut path: &[u8], predicate: F) -> Result<&'static Inode>
where
    F: Fn(&'static Inode) -> Result<&'static Inode>,
{
    if path.is_empty() {
        return Err("path empty");
    }
    let mut ip = if path[0] == b'/' {
        let sb = unsafe { &SUPERBLOCK };
        Inode::get(param::ROOTDEV, ROOTINO, sb)?
    } else {
        proc::myproc().get_cwd().dup()
    };
    while let Some((name, rest)) = skip_elem(path) {
        path = rest;
        ip = ip.with_putlock(|ip| {
            is_dir(ip)?;
            predicate(ip.dir_lookup(name)?)
        })?;
    }
    Ok(ip)
}

pub fn namei(path: &[u8]) -> Result<&'static Inode> {
    namex(path, |ip| Ok(ip))
}

pub fn namei_parent(path: &[u8]) -> (Result<&'static Inode>, &[u8]) {
    let (path, file) = split_name(path);
    (namex(path, |ip| is_dir(ip)), file)
}

pub fn split_name(path: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = path.iter().rposition(|b| *b == b'/') {
        (&path[..pos], &path[pos + 1..])
    } else {
        (&b""[..], path)
    }
}

#[cfg(test)]
mod split_name_tests {
    #[test]
    fn split_name_works() {
        use super::split_name;
        assert_eq!(split_name(&b"a/b"[..]), (&b"a"[..], &b"b"[..]));
        assert_eq!(split_name(&b"a/"[..]), (&b"a"[..], &b""[..]));
        assert_eq!(split_name(&b"/"[..]), (&b""[..], &b""[..]));
        assert_eq!(split_name(&b"/c"[..]), (&b""[..], &b"c"[..]));
    }
}

// The File interface for disk files.
impl file::Like for Inode {
    fn close(&self) {
        fslog::with_op(|| {
            self.put().expect("close failed");
        });
    }

    fn stat(&self) -> Result<Stat> {
        Ok(self.with_lock(Inode::stati))
    }

    fn read(&self, file: &File, buf: &mut [u8]) -> Result<usize> {
        self.with_lock(|ip| {
            let r = ip.readi(buf, file.off() as u64)?;
            file.inc_off(r);
            Ok(r)
        })
    }

    fn write(&self, file: &File, buf: &[u8]) -> Result<usize> {
        const MAX: usize = ((param::MAXOPBLOCKS - 1 - 1 - 2) / 2) * BSIZE;
        let mut i = 0;
        while i < buf.len() {
            let n = cmp::min(buf.len() - i, MAX);
            i += fslog::with_op(|| {
                self.with_lock(|ip| {
                    let r = ip.writei(&buf[i..i + n], file.off() as u64)?;
                    file.inc_off(r);
                    Ok(r)
                })
            })?;
        }
        Ok(i)
    }
}
