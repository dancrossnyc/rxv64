use crate::arch::{self, Page};
use crate::file;
use crate::proc::{self, myproc};
use crate::spinlock::SpinMutex as Mutex;
use crate::{FromZeros, Result};
use core::mem;
use static_assertions::const_assert;

#[repr(C)]
pub struct Pipe {
    nread: usize,
    nwrite: usize,
    data: *mut Page,
    read_open: bool,
    write_open: bool,
}
unsafe impl FromZeros for Pipe {}

impl Pipe {
    pub fn read_chan(&self) -> usize {
        &self.nread as *const _ as usize
    }

    pub fn write_chan(&self) -> usize {
        &self.nwrite as *const _ as usize
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { self.data.as_ref().unwrap().as_slice() }
    }

    fn as_mut(&mut self) -> &mut [u8] {
        unsafe { self.data.as_mut().unwrap().as_mut() }
    }

    pub fn is_empty(&self) -> bool {
        self.nread == self.nwrite
    }

    pub fn readable(&self) -> bool {
        !self.is_empty() || !self.write_open
    }

    pub fn read_byte(&mut self) -> u8 {
        assert!(!self.is_empty());
        let buf = self.as_slice();
        let b = buf[self.nread & buf.len()];
        self.nread = self.nread.wrapping_add(1);
        b
    }

    pub fn is_full(&self) -> bool {
        let buf = self.as_slice();
        self.nread + buf.len() == self.nwrite
    }

    pub fn broken(&self) -> bool {
        !self.read_open
    }

    pub fn write_byte(&mut self, b: u8) {
        assert!(!self.is_full());
        let nwrite = self.nwrite;
        let buf = self.as_mut();
        let len = buf.len();
        buf[nwrite % len] = b;
        self.nwrite = self.nwrite.wrapping_add(1);
    }
}

#[repr(transparent)]
pub struct PipeReader<'a> {
    pipe: &'a Mutex<Pipe>,
}

impl<'a> file::Like for PipeReader<'a> {
    fn close(&self) {
        let closed = self.pipe.with_lock(|pipe| {
            pipe.read_open = false;
            proc::wakeup(pipe.write_chan());
            pipe.write_open
        });
        if closed {}
    }

    fn read(&self, _file: &file::File, buf: &mut [u8]) -> Result<usize> {
        self.pipe.with_lock(|pipe| {
            while !pipe.readable() {
                if myproc().dead() {
                    return Err("broken pipe");
                }
                myproc().sleep(pipe.read_chan(), self.pipe);
            }
            let mut k = 0;
            while k < buf.len() && !pipe.is_empty() {
                buf[k] = pipe.read_byte();
                k += 1;
            }
            proc::wakeup(pipe.write_chan());
            Ok(k)
        })
    }
}

#[repr(transparent)]
pub struct PipeWriter<'a> {
    pipe: &'a Mutex<Pipe>,
}

impl<'a> file::Like for PipeWriter<'a> {
    fn close(&self) {
        let closed = self.pipe.with_lock(|pipe| {
            pipe.write_open = false;
            proc::wakeup(pipe.read_chan());
            pipe.read_open
        });
        if closed {}
    }

    fn write(&self, _file: &file::File, buf: &[u8]) -> Result<usize> {
        self.pipe.with_lock(|pipe| {
            for b in buf.iter() {
                while pipe.is_full() {
                    if pipe.broken() {
                        return Err("broken pipe");
                    }
                    proc::wakeup(pipe.read_chan());
                    myproc().sleep(pipe.write_chan(), self.pipe);
                }
                pipe.write_byte(*b);
            }
            proc::wakeup(pipe.read_chan());
            Ok(buf.len())
        })
    }
}

#[repr(C)]
struct PipeAlloc<'a> {
    pipe: Mutex<Pipe>,
    reader: PipeReader<'a>,
    writer: PipeWriter<'a>,
}

const SLAB_NPIPES: usize = (arch::PAGE_SIZE - 64) / mem::size_of::<PipeAlloc>();

#[repr(C, align(4096))]
struct PipeSlab<'a> {
    next: *mut PipeSlab<'a>,
    _padding: [u64; 2],
    bitmap: u64,
    pipes: [PipeAlloc<'a>; SLAB_NPIPES],
}
const_assert!(SLAB_NPIPES <= 64);
unsafe impl FromZeros for PipeSlab<'_> {}

pub unsafe fn init() {
    crate::println!("SLAB_NPIPES = {}", SLAB_NPIPES);
    crate::println!("PipeAlloc size = {}", mem::size_of::<PipeAlloc>());
}

impl<'a> PipeSlab<'a> {
    #[allow(dead_code)]
    pub fn alloc(&mut self) -> Option<(&PipeReader, &PipeWriter)> {
        const MASK: u64 = (1 << SLAB_NPIPES) - 1;
        if self.bitmap & MASK == MASK {
            return None;
        }
        None
    }
}
