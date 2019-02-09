use crate::arch;
use crate::file;
use crate::fs;
use crate::initcode;
use crate::kalloc;
use crate::kmem;
use crate::param;
use crate::spinlock::{without_intrs, SpinMutex as Mutex};
use crate::syscall;
use crate::vm;
use crate::Result;
use core::cell::{Cell, Ref, RefCell, RefMut};
use core::cmp;
use core::fmt;
use core::intrinsics::volatile_copy_memory;
use core::mem::size_of;
use core::ptr::{null_mut, write_volatile};
use core::slice;
use core::sync::atomic::AtomicBool;
use seq_macro::seq;
use static_assertions::const_assert_eq;

// XXX(cross): Remove this after Rust RFC 2203 is implemented.
// For the time being, we have to keep the manifest in the
// seq!() invocation in sync with NPROC.
const_assert_eq!(param::NPROC, 256);
static PROCS: Mutex<[Proc; param::NPROC]> =
    Mutex::new("procs", seq!(N in 0..256 { [#(Proc::new(),)*] }));

static mut INIT_PROC: usize = 0;

pub unsafe fn init(kpgtbl: &vm::PageTable) {
    let page = make_init_user_page(initcode::start_init_slice());
    let pgtbl = kpgtbl.dup_kern().expect("init address space alloc failed");
    let perms = vm::PageFlags::USER | vm::PageFlags::WRITE;
    pgtbl
        .map_to(kmem::ref_to_phys(page), 0, perms)
        .expect("init code map failed");

    let p = alloc().expect("allocating init proc failed");
    p.set_per_proc_data(p.as_chan(), b"init", pgtbl);
    p.set_size(arch::PAGE_SIZE);
    p.context_mut().set_return(firstret);

    INIT_PROC = p.as_chan();

    PROCS.with_lock(|_| p.set_state(ProcState::RUNNABLE));
}

fn make_init_user_page(init_code: &[u8]) -> &'static mut arch::Page {
    let page = kalloc::alloc().expect("init user alloc failed");
    page.clear();
    unsafe {
        volatile_copy_memory(
            page.as_mut().as_mut_ptr(),
            init_code.as_ptr(),
            init_code.len(),
        );
    }
    page
}

fn init_chan() -> usize {
    let ip = unsafe { INIT_PROC };
    assert_ne!(ip, 0);
    ip
}

// The first PID assigned will be 1, which is well-known
// for being reserved for init.
fn next_pid() -> u32 {
    use core::sync::atomic::{AtomicU32, Ordering};
    static PID: AtomicU32 = AtomicU32::new(1);
    PID.fetch_add(1, Ordering::Relaxed)
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProcState {
    UNUSED,
    EMBRYO,
    SLEEPING(usize),
    RUNNABLE,
    RUNNING,
    ZOMBIE,
}

#[derive(Debug)]
pub struct PerProc {
    pgtbl: Option<&'static mut vm::PageTable>,
    kstack: Option<&'static mut arch::Page>,
    pid: u32,
    parent: Option<usize>,
    context: *mut arch::Context,
    name: [u8; 16],
}

impl PerProc {
    pub const fn new() -> PerProc {
        PerProc {
            pgtbl: None,
            kstack: None,
            pid: 0,
            parent: None,
            context: null_mut(),
            name: [0; 16],
        }
    }

    pub fn set_name(&mut self, name: &[u8]) {
        let len = cmp::min(name.len(), self.name.len());
        unsafe {
            volatile_copy_memory(self.name.as_mut_ptr(), name.as_ptr(), len);
        }
    }

    pub fn context_ptr(&self) -> *const arch::Context {
        self.context
    }

    pub fn context_mut_ptr(&mut self) -> *mut arch::Context {
        self.context
    }

    pub fn mut_ptr_to_context_ptr(&mut self) -> *mut *mut arch::Context {
        &mut self.context
    }
}

pub struct Proc {
    state: Cell<ProcState>,
    killed: AtomicBool,
    data: RefCell<PerProc>,
    size: Cell<usize>,
    files: RefCell<[Option<&'static file::File>; param::NOFILE]>,
    cwd: Cell<Option<&'static fs::Inode>>,
}

impl fmt::Debug for Proc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", self as *const _ as usize)
    }
}

impl Proc {
    pub const fn new() -> Proc {
        Proc {
            state: Cell::new(ProcState::UNUSED),
            killed: AtomicBool::new(false),
            data: RefCell::new(PerProc::new()),
            size: Cell::new(0),
            files: RefCell::new([None; param::NOFILE]),
            cwd: Cell::new(None),
        }
    }

    pub fn data(&self) -> Ref<PerProc> {
        self.data.borrow()
    }

    fn data_mut(&self) -> RefMut<PerProc> {
        self.data.borrow_mut()
    }

    pub fn pid(&self) -> u32 {
        self.data().pid
    }

    pub fn state(&self) -> ProcState {
        self.state.get()
    }

    pub fn set_state(&self, state: ProcState) {
        self.state.set(state);
    }

    pub fn size(&self) -> usize {
        self.size.get()
    }

    pub fn get_cwd(&self) -> &'static fs::Inode {
        self.cwd.get().expect("proc with no pwd")
    }

    pub fn set_size(&self, size: usize) {
        self.size.set(size);
    }

    pub fn kill(&self) {
        use core::sync::atomic::Ordering;
        self.killed.store(true, Ordering::Relaxed)
    }

    pub fn resurrect(&self) {
        use core::sync::atomic::Ordering;
        self.killed.store(false, Ordering::Relaxed)
    }

    pub fn dead(&self) -> bool {
        use core::sync::atomic::Ordering;
        self.killed.load(Ordering::Relaxed)
    }

    pub fn context(&self) -> &arch::Context {
        unsafe { self.data().context_ptr().as_ref().expect("bad stack") }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn context_mut(&self) -> &mut arch::Context {
        unsafe {
            self.data_mut()
                .context_mut_ptr()
                .as_mut()
                .expect("bad stack")
        }
    }

    pub fn mut_ptr_to_context_ptr(&self) -> *mut *mut arch::Context {
        self.data_mut().mut_ptr_to_context_ptr()
    }

    pub fn parent(&self) -> usize {
        self.data().parent.unwrap_or(0)
    }

    pub fn kstack_top(&self) -> usize {
        let data = self.data();
        let pg = data.kstack.as_ref().expect("kstack");
        unsafe { (*pg as *const arch::Page).add(1) as usize }
    }

    pub fn set_parent(&self, parent: usize) {
        self.data_mut().parent = Some(parent)
    }

    pub fn initialized(&self) -> bool {
        self.state() != ProcState::UNUSED && self.state() != ProcState::EMBRYO
    }

    pub fn as_chan(&self) -> usize {
        self as *const _ as usize
    }

    pub fn pgtbl(&self) -> &vm::PageTable {
        let data = self.data();
        let pgtbl = data.pgtbl.as_ref().expect("pgtbl");
        unsafe { &*(*pgtbl as *const vm::PageTable) }
    }

    pub fn dup_pgtbl(&self) -> Option<&'static mut vm::PageTable> {
        self.pgtbl().dup(self.size())
    }

    pub fn mark_unused(&self) {
        PROCS.with_lock(|_| self.set_state(ProcState::UNUSED));
    }

    pub fn set_per_proc_data(&self, parent: usize, name: &[u8], pgtbl: &'static mut vm::PageTable) {
        let mut pd = self.data_mut();
        pd.pgtbl = Some(pgtbl);
        pd.parent = Some(parent);
        pd.set_name(name);
    }

    pub fn fork(&self) -> Option<u32> {
        let np = alloc()?;
        let mut pd = np.data.borrow_mut();
        let pgtbl = self.dup_pgtbl();
        if pgtbl.is_none() {
            kalloc::free(pd.kstack.take().unwrap());
            np.mark_unused();
            return None;
        }
        pd.pgtbl = pgtbl;
        unsafe {
            np.context_mut().set_return(forkret);
        }
        let ctx_raw = self.kstack_top() - size_of::<arch::Context>();
        let ctx = unsafe { &*(ctx_raw as *const arch::Context) };

        let ctx_raw = np.kstack_top() - size_of::<arch::Context>();
        let np_ctx = unsafe { &mut *(ctx_raw as *mut arch::Context) };
        unsafe {
            write_volatile(np_ctx, *ctx);
        }
        let mut nfiles = np.files.borrow_mut();
        let files = self.files.borrow();
        for (k, maybe_file) in files.iter().enumerate() {
            use crate::file::File;
            nfiles[k] = maybe_file.map(File::dup);
        }

        Some(pd.pid)
    }

    pub fn adjsize(&self, delta: isize) -> Result<usize> {
        let mut data = self.data_mut();
        let pgtbl = data.pgtbl.as_mut().expect("pgtbl");
        let old_size = self.size();
        let new_size = old_size.wrapping_add(delta as usize);
        if delta < 0 {
            if new_size > old_size {
                return Err("grow: underflow");
            }
            pgtbl.dealloc_user(old_size, new_size)?;
        } else {
            if old_size > new_size {
                return Err("grow: overflow");
            }
            let perms = vm::PageFlags::USER | vm::PageFlags::WRITE;
            pgtbl.alloc_user(old_size, new_size, perms)?;
        }
        self.set_size(new_size);
        unsafe {
            vm::switch(pgtbl);
        }
        Ok(new_size)
    }

    pub fn fetch_usize(&self, off: usize) -> Option<usize> {
        if off >= self.size() || off + core::mem::size_of::<usize>() >= self.size() {
            return None;
        }
        #[allow(clippy::cast_ptr_alignment)]
        let ptr = off as *const usize;
        Some(unsafe { core::ptr::read_unaligned(ptr) })
    }

    pub fn fetch_str(&self, off: usize) -> Option<&[u8]> {
        if off >= self.size() {
            return None;
        }
        let mem = unsafe { slice::from_raw_parts(off as *const u8, self.size() - off) };
        let pos = mem.iter().position(|b| *b == 0)?;
        Some(&mem[..pos])
    }

    pub fn fetch_slice(&self, off: usize, len: usize) -> Option<&[u8]> {
        if off >= self.size() || len >= self.size() - off {
            return None;
        }
        Some(unsafe { slice::from_raw_parts(off as *const u8, len) })
    }

    pub fn fetch_slice_mut(&self, off: usize, len: usize) -> Option<&mut [u8]> {
        if off >= self.size() || len >= self.size() - off {
            return None;
        }
        Some(unsafe { slice::from_raw_parts_mut(off as *mut u8, len) })
    }

    // Exit the current process.  Does not return.
    // An exited process remains in the zombie state
    // until its parent calls wait() to find out it exited.
    pub fn exit(&self) -> ! {
        assert_ne!(self.as_chan(), init_chan(), "init exiting");
        // Close open files.
        for file in self.files.borrow_mut().iter_mut().filter(|f| f.is_some()) {
            let file = file.take();
            file.unwrap().close();
        }

        use crate::file::Like;
        self.cwd.get().unwrap().close();

        let procs = PROCS.lock();
        wakeup1(&procs[..], self.parent());
        for p in procs.iter().filter(|p| p.initialized()) {
            if p.parent() == self.as_chan() {
                p.set_parent(init_chan());
                if p.state() == ProcState::ZOMBIE {
                    wakeup1(&procs[..], p.as_chan());
                }
            }
        }
        self.set_state(ProcState::ZOMBIE);
        self.sched();
        unsafe { core::intrinsics::unreachable() };
    }

    // Wait for a child process to exit and return its pid.
    // Return None if this process has no children.
    pub fn wait(&self) -> Option<u32> {
        let (pid, zkstack, zpgtbl) = self.wait1()?;
        kalloc::free(zkstack); // XXX plock held?
        vm::free(zpgtbl); // XXX plock held?
        Some(pid)
    }

    fn wait1(&self) -> Option<(u32, &mut arch::Page, &mut vm::PageTable)> {
        let procs = PROCS.lock();
        loop {
            let mut have_kids = false;
            for p in procs.iter().filter(|p| p.initialized()) {
                if p.parent() != self.as_chan() {
                    continue;
                }
                have_kids = true;
                if p.state() == ProcState::ZOMBIE {
                    let mut pd = p.data_mut();
                    let zkstack = pd.kstack.take().expect("stackless zombie");
                    let zpgtbl = pd.pgtbl.take().expect("stranded zombie");
                    let pid = pd.pid;
                    pd.pid = 0;
                    pd.parent = None;
                    pd.name = [0; 16];
                    p.resurrect();
                    p.set_size(0);
                    p.set_state(ProcState::UNUSED);
                    return Some((pid, zkstack, zpgtbl));
                }
            }
            if !have_kids || self.dead() {
                return None;
            }
            self.sleep(self.as_chan(), &PROCS);
        }
    }

    pub fn sleep<T>(&self, chan: usize, lock: &Mutex<T>) {
        let lock_procs = lock as *const _ as usize != &PROCS as *const _ as usize;
        if lock_procs {
            PROCS.acquire();
            lock.release();
        }
        self.set_state(ProcState::SLEEPING(chan));
        self.sched();
        if lock_procs {
            PROCS.release();
            lock.acquire();
        }
    }

    pub fn sched(&self) {
        assert!(PROCS.holding(), "sched proc lock");
        assert_eq!(arch::mycpu().nintr_disable(), 1, "sched locks");
        assert_ne!(self.state(), ProcState::RUNNING, "sched running");
        assert!(!arch::is_intr_enabled(), "sched interruptible");
        let intr_status = arch::mycpu().saved_intr_status();
        unsafe {
            swtch(self.mut_ptr_to_context_ptr(), arch::mycpu().scheduler());
        }
        arch::mycpu_mut().reset_saved_intr_status(intr_status);
    }

    pub fn sched_yield(&self) {
        PROCS.with_lock(|_| {
            self.set_state(ProcState::RUNNABLE);
            self.sched();
        });
    }

    pub fn get_fd(&self, fd: usize) -> Option<&file::File> {
        let files = self.files.borrow();
        if fd >= files.len() {
            None
        } else {
            files[fd]
        }
    }

    pub fn alloc_fd(&self, file: &'static file::File) -> Option<usize> {
        let mut files = self.files.borrow_mut();
        for (k, entry) in files.iter_mut().enumerate() {
            if entry.is_none() {
                *entry = Some(file);
                return Some(k);
            }
        }
        None
    }

    pub fn free_fd(&self, fd: usize) -> Option<&file::File> {
        let mut files = self.files.borrow_mut();
        if fd >= files.len() {
            None
        } else {
            files[fd].take()
        }
    }
}

pub fn yield_if_running() {
    if let Some(proc) = try_myproc() {
        if proc.state() == ProcState::RUNNING {
            proc.sched_yield();
        }
    }
}

extern "C" {
    fn swtch(from: *mut *mut arch::Context, to: &arch::Context);
}

pub fn scheduler() {
    loop {
        unsafe { arch::intr_enable() };
        let procs = PROCS.lock();
        for p in procs.iter().filter(|p| p.state() == ProcState::RUNNABLE) {
            p.set_state(ProcState::RUNNING);
            arch::mycpu_mut().set_proc(p);
            unsafe {
                vm::switch(p.pgtbl());
                swtch(arch::mycpu_mut().mut_ptr_to_scheduler_ptr(), p.context());
                vm::switch(&crate::KPGTBL);
            }
            arch::mycpu_mut().clear_proc();
        }
        arch::cpu_relax();
    }
}

// Disable interrupts so that we are not rescheduled
// while reading proc from the cpu structure
pub fn try_myproc() -> Option<&'static Proc> {
    without_intrs(|| arch::mycpu().proc())
}

pub fn myproc() -> &'static Proc {
    try_myproc().expect("myproc called with no proc")
}

extern "C" fn forkret() -> u32 {
    PROCS.release();
    0
}

extern "C" fn firstret() -> u32 {
    use crate::fslog;
    PROCS.release();
    unsafe {
        fs::init(param::ROOTDEV);
        fslog::init(param::ROOTDEV, fs::superblock());
    }
    0
}

fn alloc() -> Option<&'static Proc> {
    fn find_unused() -> Option<&'static Proc> {
        let procs = PROCS.lock();
        let proc = procs.iter().find(|p| p.state() == ProcState::UNUSED)?;
        proc.set_state(ProcState::EMBRYO);
        Some(unsafe { &*(proc as *const Proc) })
    }
    let p = find_unused()?;
    let maybe_stack = kalloc::alloc();
    if maybe_stack.is_none() {
        // We lock because p.set_state() may not be atomic,
        // and there could be another core examining the Cell
        // holding state while we're updating.
        p.mark_unused();
        return None;
    }
    let stack = maybe_stack.unwrap();
    stack.clear();
    let sp = unsafe {
        let sp = (stack.as_ptr_mut()).add(1) as *mut usize;
        let ctx_size = size_of::<arch::Context>() / size_of::<usize>();
        let sp = sp.sub(ctx_size);
        let sp = sp.sub(1);
        write_volatile(sp, syscall::syscallret as usize);
        let sp = sp.sub(ctx_size);
        let ctx = &mut *(sp as *mut arch::Context);
        ctx.set_stack(sp as usize as u64);
        sp
    };
    let mut pd = p.data.borrow_mut();
    pd.context = sp as *mut arch::Context;
    pd.pid = next_pid();
    pd.kstack = Some(stack);
    Some(p)
}

pub fn wakeup(channel: usize) {
    let procs = PROCS.lock();
    wakeup1(&procs[..], channel);
}

pub fn wakeup1(procs: &[Proc], channel: usize) {
    procs
        .iter()
        .filter(|p| p.state() == ProcState::SLEEPING(channel))
        .for_each(|p| p.set_state(ProcState::RUNNABLE));
}

// Kill the process with the given pid.
// Process won't exit until it returns
// to user space (see trap in trap.c).
pub fn kill(pid: u32) -> Option<u32> {
    let procs = PROCS.lock();
    for p in procs.iter() {
        if p.pid() == pid {
            p.kill();
            if let ProcState::SLEEPING(_) = p.state() {
                p.set_state(ProcState::RUNNABLE);
            }
            return Some(pid);
        }
    }
    None
}
