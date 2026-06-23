use crate::Result;
use crate::arch;
use crate::file;
use crate::fs;
use crate::initcode;
use crate::kalloc;
use crate::kmem;
use crate::param;
use crate::param::{USEREND, USERSTACK};
use crate::spinlock::{SpinMutex as Mutex, without_intrs};
use crate::syscall;
use crate::vm;
use crate::volatile;
use core::cell::{Cell, RefCell};
use core::cmp;
use core::fmt;
use core::mem::size_of;
use core::ptr::{self, null_mut};
use core::slice;
use core::sync::atomic::AtomicBool;

static PROCS: Mutex<[Proc; param::NPROC]> =
    Mutex::new("procs", [const { Proc::new() }; param::NPROC]);

static mut INIT_PROC: usize = 0;

pub unsafe fn init(kpgtbl: &vm::PageTable) {
    let page = make_init_user_page(initcode::start_init_slice());
    let mut pgtbl = kpgtbl.dup_kern().expect("init address space alloc failed");
    let perms = vm::PageFlags::USER | vm::PageFlags::WRITE;
    pgtbl
        .map_to(kmem::ref_to_phys(page), 0, perms)
        .expect("init code map failed");
    alloc(|p: &Proc| {
        {
            let mut pd = p.data.borrow_mut();
            pd.pgtbl = Some(pgtbl);
            pd.set_name(b"init");
        }
        p.set_parent(p.as_chan());
        p.set_size(arch::PAGE_SIZE);
        unsafe {
            p.context_mut().set_return(firstret);
            INIT_PROC = p.as_chan();
            p.user_context_mut().set_flags(arch::RFlags::INTR_EN);
        }
        p.set_state(ProcState::RUNNABLE);
        Some(())
    })
    .expect("allocating init proc failed");
}

fn make_init_user_page(init_code: &[u8]) -> &'static mut arch::Page {
    let page = kalloc::alloc().expect("init user alloc failed");
    let len = init_code.len();
    volatile::copy_slice(&mut page.as_mut()[..len], init_code);
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
    ZOMBIE(i32),
}

#[derive(Debug)]
pub struct PerProc {
    pgtbl: Option<vm::PageTable>,
    kstack: Option<&'static mut arch::Page>,
    context: *mut arch::Context,
    name: [u8; 16],
}

impl PerProc {
    pub const fn new() -> PerProc {
        PerProc {
            pgtbl: None,
            kstack: None,
            context: null_mut(),
            name: [0; 16],
        }
    }

    pub fn set_name(&mut self, name: &[u8]) {
        let len = cmp::min(name.len(), self.name.len());
        volatile::copy_slice(&mut self.name[..len], &name[..len]);
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
    pid: Cell<u32>,
    parent: Cell<Option<usize>>,
    killed: AtomicBool,
    data: RefCell<PerProc>,
    size: Cell<usize>,
    files: RefCell<[Option<&'static file::File>; param::NOFILE]>,
    cwd: Cell<Option<&'static fs::Inode>>,
}

impl fmt::Debug for Proc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", (self as *const Self).addr())
    }
}

impl Proc {
    pub const fn new() -> Proc {
        Proc {
            state: Cell::new(ProcState::UNUSED),
            pid: Cell::new(0),
            parent: Cell::new(None),
            killed: AtomicBool::new(false),
            data: RefCell::new(PerProc::new()),
            size: Cell::new(0),
            files: RefCell::new([None; param::NOFILE]),
            cwd: Cell::new(None),
        }
    }

    pub fn with_pgtbl<F, U>(&self, thunk: F) -> U
    where
        F: FnOnce(&mut vm::PageTable) -> U,
    {
        let mut data = self.data.borrow_mut();
        let pgtbl = data.pgtbl.as_mut().expect("pgtbl");
        thunk(pgtbl)
    }

    pub fn pid(&self) -> u32 {
        self.pid.get()
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

    pub fn cwd(&self) -> &'static fs::Inode {
        self.cwd.get().expect("proc with no pwd")
    }

    pub fn set_cwd(&self, ip: &'static fs::Inode) {
        self.cwd.set(Some(ip));
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
        unsafe {
            self.data
                .borrow()
                .context_ptr()
                .as_ref()
                .expect("bad stack")
        }
    }

    fn user_context_addr(&self) -> usize {
        self.kstack_top() - size_of::<arch::TrapFrame>()
    }

    pub unsafe fn user_context(&self) -> &arch::TrapFrame {
        let raw = self.user_context_addr();
        unsafe { (raw as *const arch::TrapFrame).as_ref() }.unwrap()
    }

    #[allow(clippy::mut_from_ref)]
    pub unsafe fn user_context_mut(&self) -> &mut arch::TrapFrame {
        let raw = self.user_context_addr();
        unsafe { (raw as *mut arch::TrapFrame).as_mut() }.unwrap()
    }

    #[allow(clippy::mut_from_ref)]
    pub fn context_mut(&self) -> &mut arch::Context {
        unsafe {
            self.data
                .borrow_mut()
                .context_mut_ptr()
                .as_mut()
                .expect("bad stack")
        }
    }

    pub fn mut_ptr_to_context_ptr(&self) -> *mut *mut arch::Context {
        self.data.borrow_mut().mut_ptr_to_context_ptr()
    }

    pub fn parent(&self) -> usize {
        self.parent.get().unwrap_or(0)
    }

    pub fn kstack_top(&self) -> usize {
        let data = self.data.borrow();
        let pg = data.kstack.as_ref().expect("kstack");
        unsafe { (*pg as *const arch::Page).add(1).addr() }
    }

    pub fn set_parent(&self, parent: usize) {
        self.parent.set(Some(parent))
    }

    pub fn initialized(&self) -> bool {
        self.state() != ProcState::UNUSED && self.state() != ProcState::EMBRYO
    }

    pub fn as_chan(&self) -> usize {
        (self as *const Self).addr()
    }

    pub fn dup_pgtbl(&self) -> Option<vm::PageTable> {
        self.data.borrow().pgtbl.as_ref()?.dup(self.size())
    }

    pub unsafe fn switch_pgtbl(&self, pgtbl: vm::PageTable) -> Option<vm::PageTable> {
        unsafe {
            pgtbl.switch();
        }
        self.data.borrow_mut().pgtbl.replace(pgtbl)
    }

    pub fn fork(&self) -> Option<u32> {
        alloc(|np| -> Option<()> {
            {
                let mut pd = np.data.borrow_mut();
                let pgtbl = self.dup_pgtbl().or_else(|| {
                    kalloc::free(pd.kstack.take().unwrap());
                    np.set_state(ProcState::UNUSED);
                    None
                })?;
                pd.pgtbl = Some(pgtbl);
                pd.set_name(&self.data.borrow().name);
            }
            unsafe {
                let ctx = self.user_context();
                let nctx = np.user_context_mut();
                volatile::copy(nctx, ctx);
                np.context_mut().set_return(forkret);
            }
            np.set_parent(self.as_chan());
            np.set_size(self.size());
            let mut nfiles = np.files.borrow_mut();
            let files = self.files.borrow();
            for (k, maybe_file) in files.iter().enumerate() {
                use crate::file::File;
                nfiles[k] = maybe_file.map(File::dup);
            }
            np.set_cwd(self.cwd().dup());
            np.set_state(ProcState::RUNNABLE);
            Some(())
        })
    }

    pub fn adjsize(&self, delta: isize) -> Result<usize> {
        let old_size = self.size();
        let new_size = old_size.wrapping_add(delta as usize);
        if delta < 0 {
            if new_size > old_size {
                return Err("grow: underflow");
            }
            self.with_pgtbl(|pgtbl| pgtbl.dealloc_user(old_size, new_size))?;
        } else {
            if old_size > new_size {
                return Err("grow: overflow");
            }
            let perms = vm::PageFlags::USER | vm::PageFlags::WRITE;
            self.with_pgtbl(|pgtbl| pgtbl.alloc_user(old_size, new_size, perms))?;
        }
        self.set_size(new_size);
        unsafe {
            // Flush the TLB.
            self.data.borrow().pgtbl.as_ref().expect("pgtbl").switch();
        }
        Ok(old_size)
    }

    fn is_user_addr(&self, va: usize) -> bool {
        va < self.size() || (USERSTACK..USEREND).contains(&va)
    }

    fn user_region_end(&self, va: usize) -> Option<usize> {
        if self.is_user_addr(va) {
            Some(if va < self.size() {
                self.size()
            } else {
                USEREND
            })
        } else {
            None
        }
    }

    pub fn fetch_usize(&self, off: usize) -> Option<usize> {
        let rend = self.user_region_end(off)?;
        if size_of::<usize>() > rend - off {
            return None;
        }
        #[allow(clippy::cast_ptr_alignment)]
        let ptr = off as *const usize;
        Some(unsafe { ptr::read_unaligned(ptr) })
    }

    pub fn fetch_str(&self, off: usize) -> Option<&[u8]> {
        let rend = self.user_region_end(off)?;
        let mem = unsafe { slice::from_raw_parts(off as *const u8, rend - off) };
        let pos = mem.iter().position(|b| *b == 0)?;
        Some(&mem[..pos])
    }

    pub fn fetch_slice(&self, off: usize, len: usize) -> Option<&[u8]> {
        let rend = self.user_region_end(off)?;
        if len > rend - off {
            return None;
        }
        Some(unsafe { slice::from_raw_parts(off as *const u8, len) })
    }

    pub fn fetch_slice_mut(&self, off: usize, len: usize) -> Option<&mut [u8]> {
        let rend = self.user_region_end(off)?;
        if len > rend - off {
            return None;
        }
        Some(unsafe { slice::from_raw_parts_mut(off as *mut u8, len) })
    }

    pub fn fetch_ptr_mut<T>(&self, off: usize, len: usize) -> Option<*mut T> {
        let rend = self.user_region_end(off)?;
        if (len * size_of::<T>()) > rend - off {
            return None;
        }
        #[allow(clippy::cast_ptr_alignment)]
        Some(off as *mut T)
    }

    // Exit the current process.  Does not return.
    // An exited process remains in the zombie state
    // until its parent calls wait() to find out it exited.
    pub fn exit(&self, status: i32) -> ! {
        assert_ne!(self.as_chan(), init_chan(), "init exiting");
        // Close open files.
        for file in self.files.borrow_mut().iter_mut().filter(|f| f.is_some()) {
            let file = file.take();
            file.unwrap().close();
        }

        crate::fslog::with_op(|| self.cwd.take().unwrap().put().expect("iput cwd"));

        let procs = PROCS.lock();
        wakeup1(&procs[..], self.parent());
        for p in procs.iter().filter(|&p| p.initialized()) {
            if p.parent() == self.as_chan() {
                p.set_parent(init_chan());
                if let ProcState::ZOMBIE(_) = p.state() {
                    wakeup1(&procs[..], init_chan());
                }
            }
        }
        self.set_state(ProcState::ZOMBIE(status));
        self.sched();
        core::unreachable!();
    }

    // Wait for a child process to exit and return its pid.
    // Return None if this process has no children.
    pub fn wait(&self, status_ptr: usize) -> Option<u32> {
        let statusp = self.fetch_ptr_mut::<i32>(status_ptr, 1)?;
        let (pid, status, zkstack, zpgtbl) = self.wait1()?;
        kalloc::free(zkstack); // XXX plock held?
        drop(zpgtbl); // XXX plock held?
        if statusp.is_null() {
            unsafe {
                ptr::write(statusp, status);
            }
        }
        Some(pid)
    }

    fn wait1(&self) -> Option<(u32, i32, &mut arch::Page, vm::PageTable)> {
        let procs = PROCS.lock();
        loop {
            let mut have_kids = false;
            for p in procs.iter().filter(|&p| p.initialized()) {
                if p.parent() != self.as_chan() {
                    continue;
                }
                have_kids = true;
                if let ProcState::ZOMBIE(status) = p.state() {
                    let zkstack;
                    let zpgtbl;
                    {
                        let mut pd = p.data.borrow_mut();
                        zkstack = pd.kstack.take().expect("stackless zombie");
                        zpgtbl = pd.pgtbl.take().expect("stranded zombie");
                        pd.name = [0; 16];
                    }
                    let pid = p.pid.take();
                    p.parent.set(None);
                    p.resurrect();
                    p.set_size(0);
                    p.set_state(ProcState::UNUSED);
                    return Some((pid, status, zkstack, zpgtbl));
                }
            }
            if !have_kids || self.dead() {
                return None;
            }
            self.sleep(self.as_chan(), &PROCS);
        }
    }

    pub fn sleep<T>(&self, chan: usize, lock: &Mutex<T>) {
        let lock_procs = !ptr::eq(lock, &PROCS as *const _ as *const Mutex<T>);
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
        self.set_state(ProcState::RUNNABLE);
        self.sched();
    }

    pub fn get_fd(&self, fd: usize) -> Option<&file::File> {
        let files = self.files.borrow();
        if fd >= files.len() { None } else { files[fd] }
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
        PROCS.with_lock(|_| {
            if proc.state() == ProcState::RUNNING {
                proc.sched_yield();
            }
        });
    }
}

pub fn die_if_dead() {
    if let Some(proc) = try_myproc()
        && proc.dead()
    {
        proc.exit(1);
    }
}

unsafe extern "C" {
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
                p.data.borrow().pgtbl.as_ref().unwrap().switch();
                swtch(arch::mycpu_mut().mut_ptr_to_scheduler_ptr(), p.context());
                crate::kpgtbl().switch();
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
        myproc().set_cwd(fs::namei(b"/").expect("root filesystem exists"));
    }
    0
}

fn alloc<F>(thunk: F) -> Option<u32>
where
    F: FnOnce(&Proc) -> Option<()>,
{
    fn init_proc(p: &Proc, stack: &'static mut arch::Page) -> u32 {
        p.set_state(ProcState::EMBRYO);
        let mut pd = p.data.borrow_mut();
        pd.context = unsafe {
            let sp = (stack.as_ptr_mut()).add(1);
            let sp = sp as *mut usize;
            // Allocate stack space for the syscall context.
            let sp = sp.sub(size_of::<arch::TrapFrame>() / size_of::<usize>());
            // Arrange for the scheduler to return to `syscallret`
            // and allocate space for the kernel scheduler context.
            let sp = sp.sub(1);
            let syscallret = syscall::syscallret as *const ();
            volatile::write(sp.as_mut().unwrap(), syscallret.addr());
            let sp = sp.sub(size_of::<arch::Context>() / size_of::<usize>());
            let ctx = (sp as *mut arch::Context).as_mut().unwrap();
            ctx.set_stack(sp.addr() as u64);
            ctx as *mut arch::Context
        };
        pd.kstack = Some(stack);
        let pid = next_pid();
        p.pid.set(pid);
        pid
    }
    let stack = kalloc::alloc()?;
    let procs = PROCS.lock();
    let Some(p) = procs.iter().find(|&p| p.state() == ProcState::UNUSED) else {
        kalloc::free(stack);
        return None;
    };
    let pid = init_proc(p, stack);
    thunk(p)?;
    Some(pid)
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

pub fn dump() {
    let ps = PROCS.lock();
    for p in &ps[..] {
        if let ProcState::UNUSED = p.state() {
            continue;
        }
        crate::println!("{} {:x?}", p.pid(), p.state());
    }
}
