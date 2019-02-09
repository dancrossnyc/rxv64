use crate::kmem;
use crate::proc;
use crate::spinlock::without_intrs;
use crate::trap::trap;
use crate::FromZeros;
use bitflags::bitflags;
use core::fmt;
use core::ptr;
use core::time;
use seq_macro::seq;
use zerocopy::FromBytes;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod asm {
    global_asm!(include_str!("entry.S"), options(att_syntax));
    global_asm!(include_str!("swtch.S"), options(att_syntax));
}

pub const PAGE_SIZE: usize = 4096;
#[repr(C, align(4096))]
#[derive(FromBytes)]
pub struct Page([u8; PAGE_SIZE]);
unsafe impl FromZeros for Page {}

impl Page {
    pub const fn empty() -> Page {
        Page([0; PAGE_SIZE])
    }

    pub fn clear(&mut self) {
        unsafe {
            core::intrinsics::volatile_set_memory(&mut self.0, 0u8, 1);
        }
    }

    pub fn scribble(&mut self) {
        unsafe {
            core::intrinsics::volatile_set_memory(self, 0b1010_1010u8, 1);
        }
    }

    pub fn phys_addr(&self) -> u64 {
        kmem::ref_to_phys(self)
    }

    pub fn as_ptr_mut(&mut self) -> *mut Page {
        self as *mut Page
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0[..]
    }

    pub fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0[..]
    }
}

impl fmt::Debug for Page {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", self as *const _ as usize)
    }
}

pub const fn page_round_down(value: usize) -> usize {
    value & !(PAGE_SIZE - 1)
}

pub const fn page_round_up(value: usize) -> usize {
    (PAGE_SIZE - value % PAGE_SIZE) % PAGE_SIZE + value
}

#[cfg(test)]
mod page_round_tests {
    use super::{page_round_down, page_round_up, PAGE_SIZE};
    #[test]
    fn test_page_round_down() {
        assert_eq!(page_round_down(PAGE_SIZE), PAGE_SIZE);
        assert_eq!(page_round_down(PAGE_SIZE + 1), PAGE_SIZE);
    }

    #[test]
    fn test_page_round_up() {
        assert_eq!(page_round_up(PAGE_SIZE), PAGE_SIZE);
        assert_eq!(page_round_up(PAGE_SIZE - 1), PAGE_SIZE);
        assert_eq!(page_round_up(PAGE_SIZE + 1), PAGE_SIZE * 2);
    }
}

const SMALL_STACK_SIZE: usize = 512;
#[repr(C, align(512))]
#[derive(FromBytes)]
pub struct SmallStack([u8; SMALL_STACK_SIZE]);
unsafe impl FromZeros for SmallStack {}

impl SmallStack {
    pub fn top_as_u64(&mut self) -> u64 {
        unsafe { (self as *mut SmallStack).add(SMALL_STACK_SIZE) as u64 }
    }
}

pub mod pic {
    use super::outb;
    pub fn init() {
        unsafe {
            const PIC1: u16 = 0x20;
            const PIC2: u16 = 0xA0;
            outb(PIC1 + 1, 0xFF);
            outb(PIC2 + 1, 0xFF);
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub rsp: u64,
    pub rflags: u64,
    rbp: u64,
    rbx: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
}

impl Context {
    pub unsafe fn set_return(&mut self, thunk: extern "C" fn() -> u32) {
        self.rip = thunk as usize as u64;
    }

    pub unsafe fn set_stack(&mut self, sp: u64) {
        self.rsp = sp;
    }
}

mod segment {
    use super::SmallStack;

    #[repr(transparent)]
    pub struct Desc(u64);

    #[repr(C)]
    pub struct TaskDesc([u64; 2]);

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GateDesc([u64; 2]);

    impl GateDesc {
        pub const fn empty() -> GateDesc {
            GateDesc([0, 0])
        }
    }

    const SYSTEM: u64 = 1 << (12 + 32);
    const PRESENT: u64 = 1 << (15 + 32);
    const LONG: u64 = 1 << (21 + 32);

    const TYPE_CONFORM: u64 = 0b1000 << (8 + 32);
    const TYPE_READWRITE: u64 = 0b0010 << (8 + 32);
    const TYPE_READONLY: u64 = 0b0001 << (8 + 32);

    const TYPE_DATA: u64 = TYPE_READWRITE;
    const TYPE_TEXT: u64 = TYPE_CONFORM | TYPE_READONLY;
    const TYPE_TASK_AVAIL: u64 = 0b1001 << (8 + 32);

    const TYPE_INTR_GATE: u64 = 0b1110 << (8 + 32);

    const DPL_KERN: u64 = 0b00 << (13 + 32);
    const DPL_USER: u64 = 0b11 << (13 + 32);

    pub const fn null() -> Desc {
        Desc(0)
    }
    pub const fn ktext64() -> Desc {
        Desc(SYSTEM | PRESENT | LONG | TYPE_TEXT | DPL_KERN)
    }
    pub const fn kdata64() -> Desc {
        Desc(SYSTEM | PRESENT | TYPE_DATA | DPL_KERN)
    }
    pub const fn utext64() -> Desc {
        Desc(SYSTEM | PRESENT | LONG | TYPE_TEXT | DPL_USER)
    }
    pub const fn udata64() -> Desc {
        Desc(PRESENT | TYPE_DATA | DPL_USER)
    }

    pub fn task64(task: &TaskState) -> TaskDesc {
        let base = task as *const TaskState as u64;
        let base0_upper = ((base & 0xFF00_0000) | ((base & 0x00FF_0000) >> 16)) << 32;
        let base0_lower = (base & 0xFFFF) << 16;
        let base0 = base0_upper | base0_lower;
        let limit = core::mem::size_of::<TaskState>() as u64 - 1;
        let lower = limit | base0 | PRESENT | TYPE_TASK_AVAIL | DPL_KERN;
        let upper = base >> 32;
        TaskDesc([lower, upper])
    }

    pub enum IntrStack {
        RSP0 = 0,
        NMI = 1,
        DB = 2,
        DFault = 3,
    }

    pub fn intr64(thunk: unsafe extern "C" fn() -> !, stack: IntrStack) -> GateDesc {
        let offset = thunk as usize as u64;
        let lower0_offset = offset & 0x0000_FFFF;
        let lower0 = (u64::from(KTEXT_SEL) << 16) | lower0_offset;
        let lower1_offset = (offset & 0xFFFF_0000) << 32;
        let lower1 = ((stack as u64) << 32) | lower1_offset;
        let lower = lower1 | lower0 | PRESENT | TYPE_INTR_GATE | DPL_KERN;
        let upper = offset >> 32;
        GateDesc([lower, upper])
    }

    #[repr(C, packed)]
    pub struct TaskState {
        _reserved0: u32,
        rsp0: u64,
        _rsp1: u64,
        _rsp2: u64,
        _reserved1: u32,
        _reserved2: u32,
        ist1: u64,
        ist2: u64,
        ist3: u64,
        _ist4: u64,
        _ist5: u64,
        _ist6: u64,
        _ist7: u64,
        _reserved3: u32,
        _reserved4: u32,
        _reserved5: u16,
        io_map_base: u16,
    }

    impl TaskState {
        pub fn new(
            nmi_stack: &mut SmallStack,
            db_stack: &mut SmallStack,
            dbl_flt_stack: &mut SmallStack,
        ) -> TaskState {
            TaskState {
                _reserved0: 0,
                rsp0: 0,
                _rsp1: 0,
                _rsp2: 0,
                _reserved1: 0,
                _reserved2: 0,
                ist1: nmi_stack.top_as_u64(),
                ist2: db_stack.top_as_u64(),
                ist3: dbl_flt_stack.top_as_u64(),
                _ist4: 0,
                _ist5: 0,
                _ist6: 0,
                _ist7: 0,
                _reserved3: 0,
                _reserved4: 0,
                _reserved5: 0,
                io_map_base: core::mem::size_of::<TaskState>() as u16,
            }
        }

        pub fn set_rsp0(&mut self, rsp0: u64) {
            self.rsp0 = rsp0;
        }
    }

    pub const _NULL_SEL: u16 = 0 << 3;
    pub const KTEXT_SEL: u16 = 1 << 3;
    pub const _KDATA_SEL: u16 = 2 << 3;
    pub const _UDATA_SEL: u16 = 3 << 3;
    pub const UTEXT_SEL: u16 = 4 << 3;
    pub const TASK_SEL: u16 = 6 << 3;

    #[repr(C)]
    pub struct GDT {
        null: Desc,
        ktext: Desc,
        kdata: Desc,
        udata: Desc,
        utext: Desc,
        unused: u64,
        task: TaskDesc,
    }

    impl GDT {
        pub fn new(task: &TaskState) -> GDT {
            GDT {
                null: null(),
                ktext: ktext64(),
                kdata: kdata64(),
                udata: udata64(),
                utext: utext64(),
                unused: 0,
                task: task64(task),
            }
        }
    }

    pub unsafe fn init(gdt: &GDT) {
        lgdt(gdt);
        ltr(TASK_SEL);
    }

    unsafe fn lgdt(gdt: &GDT) {
        let base = gdt as *const GDT as u64;
        const LIMIT: u16 = core::mem::size_of::<GDT>() as u16 - 1;
        asm!(r#"
            subq $16, %rsp;
            movq {}, 8(%rsp);
            movw ${}, 6(%rsp);
            lgdt 6(%rsp);
            addq $16, %rsp;
            "#, in(reg) base, const LIMIT, options(att_syntax));
    }

    unsafe fn ltr(sel: u16) {
        asm!("ltr {:x}", in(reg) sel);
    }

    pub fn star() -> u64 {
        // On 64-bit return, SYSRETQ loads the CS with
        // the value of star[48..64] + 16.  Why +16?
        // For compatibility mode with 32-bit legacy
        // systems: presumably the GDT would have 32-bit
        // entries first, then 64-bit entries.  On a 64-bit
        // only system, this doesn't isn't an issue, but
        // we still need to load the value in IA32_STAR
        // with this offset.  Interestingly, SS is loaded
        // with star[48..64] + 8.  In rxv64, this is the
        // NULL selector, which is exactly what we want.
        //
        // Note that SYSRET on Intel forces RPL on the
        // loaded selectors to 3, but AMD only does this
        // for CS and not SS.  So for portability, we
        // explicitly OR a user RPL into the STAR bits.
        // This is idempotent for CS.
        const RPL_USER: u64 = 0x3;
        ((u64::from(UTEXT_SEL) - 16) | RPL_USER) << 48 | u64::from(KTEXT_SEL) << 32
    }
}

unsafe fn lidt(idt: &IDT) {
    let base = idt as *const IDT as u64;
    const LIMIT: u16 = core::mem::size_of::<IDT>() as u16 - 1;
    asm!(r#"
        subq $16, %rsp;
        movq {}, 8(%rsp);
        movw ${}, 6(%rsp);
        lidt 6(%rsp);
        addq $16, %rsp;
        "#, in(reg) base, const LIMIT, options(att_syntax));
}

#[repr(C, align(4096))]
pub struct CPU {
    self_ptr: *mut CPU,
    ureg: u64,
    kstack: u64,
    id: u32,
    clock_freq: u64,
    scheduler: *mut Context,
    ts: segment::TaskState,
    gdt: segment::GDT,
    ncli: u32,
    saved_intr_status: bool,
    proc: *const proc::Proc,
}

impl CPU {
    #[allow(clippy::cast_ptr_alignment)]
    pub unsafe fn init(page: &mut Page, id: u32) {
        let cpu = &mut *(page as *mut Page as *mut CPU);
        let nmi_stack = &mut *(&mut page.0[1024] as *mut u8 as *mut SmallStack);
        let db_stack = &mut *(&mut page.0[1024 + 512] as *mut u8 as *mut SmallStack);
        let dbl_flt_stack = &mut *(&mut page.0[2048] as *mut u8 as *mut SmallStack);
        *cpu = CPU {
            self_ptr: cpu,
            ureg: 0,
            kstack: 0,
            id,
            clock_freq: tschz(),
            scheduler: ptr::null_mut(),
            ts: segment::TaskState::new(nmi_stack, db_stack, dbl_flt_stack),
            gdt: segment::GDT::new(&cpu.ts),
            ncli: 0,
            saved_intr_status: false,
            proc: ptr::null_mut(),
        };
        wrgsbase(cpu as *mut CPU as u64);
        segment::init(&cpu.gdt);
    }

    pub unsafe fn push_intr_disable() {
        let enabled = is_intr_enabled();
        intr_disable();
        let cpu = mycpu_mut();
        if cpu.ncli == 0 {
            cpu.saved_intr_status = enabled;
        }
        cpu.ncli += 1;
    }

    pub unsafe fn pop_intr_disable() {
        assert!(!is_intr_enabled(), "pop while interrupts are enabled");
        let cpu = mycpu_mut();
        assert!(cpu.ncli != 0, "pop_intr_disable ncli = {}", cpu.ncli);
        cpu.ncli -= 1;
        if cpu.ncli == 0 && cpu.saved_intr_status {
            intr_enable();
        }
    }

    pub fn nintr_disable(&self) -> u32 {
        self.ncli
    }

    pub fn saved_intr_status(&self) -> bool {
        self.saved_intr_status
    }

    pub fn reset_saved_intr_status(&mut self, status: bool) {
        self.saved_intr_status = status;
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn set_proc(&mut self, proc: &proc::Proc) {
        without_intrs(|| {
            self.proc = proc;
            let stack_top = proc.kstack_top() as u64;
            self.kstack = stack_top;
            self.ts.set_rsp0(stack_top);
        });
    }

    pub fn clear_proc(&mut self) {
        self.proc = ptr::null();
        self.ts.set_rsp0(0);
    }

    pub fn proc(&self) -> Option<&proc::Proc> {
        unsafe { self.proc.as_ref() }
    }

    pub fn scheduler(&self) -> &Context {
        unsafe { &*self.scheduler }
    }

    pub fn mut_ptr_to_scheduler_ptr(&mut self) -> &mut *mut Context {
        &mut self.scheduler
    }
}

pub use segment::star;

pub unsafe fn intr_disable() {
    asm!("cli");
}

pub unsafe fn intr_enable() {
    asm!("sti");
}

pub fn mycpu_id() -> u32 {
    mycpu().id()
}

pub fn mycpu() -> &'static CPU {
    unsafe {
        use core::mem::transmute;
        let base: u64;
        asm!("movq %gs:0, {}", out(reg) base, options(att_syntax));
        transmute::<u64, &CPU>(base)
    }
}

pub fn mycpu_mut() -> &'static mut CPU {
    unsafe {
        use core::mem::transmute;
        let base: u64;
        asm!("movq %gs:0, {}", out(reg) base, options(att_syntax));
        transmute::<u64, &mut CPU>(base)
    }
}

bitflags! {
    pub struct RFlags: u64 {
        const CARRY     = 1;
        const PARITY    = 1 << 2;
        const ADJUST    = 1 << 4;
        const ZERO      = 1 << 6;
        const SIGN      = 1 << 7;
        const TRAP      = 1 << 8;
        const INTR_EN   = 1 << 9;
        const DIRECTION = 1 << 10;
        const OVERFLOW  = 1 << 11;
        const IOPL      = 3 << 12;
    }
}

pub fn flags() -> RFlags {
    let raw: u64;
    unsafe {
        asm!("pushfq; popq {}", out(reg) raw, options(att_syntax));
    }
    RFlags::from_bits_truncate(raw)
}

pub fn is_intr_enabled() -> bool {
    flags().contains(RFlags::INTR_EN)
}

pub fn sfmask() -> u64 {
    (RFlags::TRAP | RFlags::INTR_EN | RFlags::DIRECTION).bits()
}

pub unsafe fn outb(port: u16, b: u8) {
    asm!("outb %al, %dx", in("al") b, in("dx") port, options(att_syntax));
}

pub unsafe fn inb(port: u16) -> u8 {
    let r: u8;
    asm!("inb %dx, %al", in("dx") port, out("al") r, options(att_syntax));
    r
}

pub unsafe fn _outw(port: u16, w: u16) {
    asm!("outw %ax, %dx", in("ax") w, in("dx") port, options(att_syntax));
}

pub unsafe fn _inw(port: u16) -> u16 {
    let r: u16;
    asm!("inw %dx, %ax", in("dx") port, out("ax") r, options(att_syntax));
    r
}

pub unsafe fn _outl(port: u16, l: u32) {
    asm!("outl %eax, %dx", in("eax") l, in("dx") port, options(att_syntax));
}

pub unsafe fn _inl(port: u16) -> u32 {
    let r: u32;
    asm!("inl %dx, %eax", in("dx") port, out("eax") r, options(att_syntax));
    r
}

pub unsafe fn load_page_table(pt: u64) {
    asm!("movq {}, %cr3", in(reg) pt, options(att_syntax));
}

pub fn fault_addr() -> usize {
    let addr: usize;
    unsafe {
        asm!("movq %cr2, %rax", out("rax") addr, options(att_syntax));
    }
    addr
}

pub fn xswap(word: &mut u64, value: u64) -> u64 {
    unsafe { core::intrinsics::atomic_xchg(word, value) }
}

// TODO(cross): Crashes the Rust compiler as of Dec, 2019.
// The RFC 2873 syntax doesn't support memory operands as
// of July 6, 2020.
//
// pub fn xswap(mut word: &mut u64, mut value: u64) -> u64 {
//     let mut word = word as *mut u64;
//     unsafe {
//         llvm_asm!("lock; xchgq %0, %1"
//              : "=r"(value), "=m"(*word)
//              : "0"(value), "m"(*word)
//              :: "volatile");
//     }
//     value
// }

pub fn cpu_relax() {
    unsafe {
        asm!("pause");
    }
}

pub unsafe fn rdmsr(index: u32) -> u64 {
    let val_lo: u32;
    let val_hi: u32;
    asm!("rdmsr", in("ecx") index, out("eax") val_lo, out("edx") val_hi, options(att_syntax));
    (u64::from(val_hi) << 32) | u64::from(val_lo)
}

pub unsafe fn wrmsr(index: u32, value: u64) {
    asm!("wrmsr",
         in("ecx") index, in("eax") value as u32, in("edx") (value >> 32) as u32,
         options(att_syntax));
}

unsafe fn tschz() -> u64 {
    if false {
        const TSC_INV_MULTIPLIER: u64 = 133_330_000; // 133.33 MHz
        const MSR_PLATFORM_INFO: u32 = 0x206;
        let platform_info = rdmsr(MSR_PLATFORM_INFO);
        let max_non_turbo_ratio = (platform_info >> 8) & 0xFF;
        max_non_turbo_ratio * TSC_INV_MULTIPLIER
    } else {
        2_000_000_000
    }
}

fn rdtsc() -> u128 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!("rdtsc", out("eax") lo, out("edx") hi, options(att_syntax));
    }
    u128::from(hi) << 32 | u128::from(lo)
}

fn nanosleep(n: u128) {
    let delta = n * u128::from(mycpu().clock_freq) / 1_000_000_000;
    let s = rdtsc();
    while rdtsc() - s < delta {
        cpu_relax();
    }
}

pub fn sleep(n: time::Duration) {
    nanosleep(n.as_nanos());
}

unsafe fn wrgsbase(w: u64) {
    asm!("wrgsbase {}", in(reg) w, options(att_syntax));
}

#[allow(dead_code)]
pub fn read_u16(bs: &[u8]) -> u16 {
    assert!(bs.len() == 2);
    let bs = [bs[0], bs[1]];
    u16::from_ne_bytes(bs)
}
pub fn read_u32(bs: &[u8]) -> u32 {
    assert!(bs.len() == 4);
    let bs = [bs[0], bs[1], bs[2], bs[3]];
    u32::from_ne_bytes(bs)
}

pub fn read_u64(bs: &[u8]) -> u64 {
    assert!(bs.len() == 8);
    let bs = [bs[0], bs[1], bs[2], bs[3], bs[4], bs[5], bs[6], bs[7]];
    u64::from_ne_bytes(bs)
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct TrapFrame {
    // Pushed by software.
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,

    // It is arguable whether we should care about
    // these registers.  x86 segmentation (aside from
    // FS and GS) isn't used once we're in long mode,
    // and rxv64 doesn't support real or compatibility
    // mode, so these are effectively unused.
    //
    // Regardless, they exist, so we save and restore
    // them.  Some kernels do this, some do not.  Note
    // that %fs and %gs are special.
    ds: u64, // Really these are u16s, but
    es: u64, // we waste a few bytes to keep
    fs: u64, // the stack aligned.  Thank
    gs: u64, // you, x86 segmentation.

    vector: u64,

    // Sometimes pushed by hardware.
    pub error: u64,

    // Pushed by hardware.
    pub rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

const TRAPFRAME_VECTOR_OFFSET: usize = 0x98;
const TRAPFRAME_CS_OFFSET: usize = 0xB0;

macro_rules! gen_stub {
    ($name:ident, $vecnum:expr) => {
        #[allow(dead_code)]
        #[link_section = ".trap"]
        #[naked]
        unsafe extern "C" fn $name() -> ! {
            asm!("pushq $0; pushq ${}; jmp {}",
                const $vecnum, sym alltraps,
                options(att_syntax, noreturn));
        }
    };
    ($name:ident, $vecnum:expr, err) => {
        #[allow(dead_code)]
        #[link_section = ".trap"]
        #[naked]
        unsafe extern "C" fn $name() -> ! {
            asm!("pushq ${}; jmp {}",
                const $vecnum, sym alltraps,
                options(att_syntax, noreturn));
        }
    };
}

macro_rules! gen_vector_stub {
    // These cases include hardware-generated error words
    // on the trap frame
    (vector8, 8) => {
        gen_stub!(vector8, 8, err);
    };
    (vector10, 10) => {
        gen_stub!(vector10, 10, err);
    };
    (vector11, 11) => {
        gen_stub!(vector11, 11, err);
    };
    (vector12, 12) => {
        gen_stub!(vector12, 12, err);
    };
    (vector13, 13) => {
        gen_stub!(vector13, 13, err);
    };
    (vector14, 14) => {
        gen_stub!(vector14, 14, err);
    };
    (vector17, 17) => {
        gen_stub!(vector17, 17, err);
    };
    // No hardware error
    ($vector:ident, $num:expr) => {
        gen_stub!($vector, $num);
    };
}

seq!(N in 0..=255 {
    gen_vector_stub!(vector#N, N);
});

#[link_section = ".trap"]
#[naked]
unsafe extern "C" fn alltraps() -> ! {
    asm!(r#"
        // Save the x86 segmentation registers.
        subq $32, %rsp
        movq $0, (%rsp);
        movw %ds, (%rsp);
        movq $0, 8(%rsp);
        movw %es, 8(%rsp);
        movq $0, 16(%rsp);
        movw %fs, 16(%rsp);
        movq $0, 24(%rsp);
        movw %gs, 24(%rsp);
        pushq %r15;
        pushq %r14;
        pushq %r13;
        pushq %r12;
        pushq %r11;
        pushq %r10;
        pushq %r9;
        pushq %r8;
        pushq %rbp;
        pushq %rdi;
        pushq %rsi;
        pushq %rdx;
        pushq %rcx;
        pushq %rbx;
        pushq %rax;
        cmpq ${ktext_sel}, {cs_offset}(%rsp);
        je 1f;
        swapgs;
        1: movq {vector_offset}(%rsp), %rdi;
        movq %rsp, %rsi;
        callq {trap};
        cmpq ${ktext_sel}, {cs_offset}(%rsp);
        je 1f;
        swapgs;
        1: popq %rax;
        popq %rbx;
        popq %rcx;
        popq %rdx;
        popq %rsi;
        popq %rdi;
        popq %rbp;
        popq %r8;
        popq %r9;
        popq %r10;
        popq %r11;
        popq %r12;
        popq %r13;
        popq %r14;
        popq %r15;
        // If necessary, %gs is restored via swapgs above.
        // %fs is special.  We ought to save it and restore
        // it, should userspace ever use green threads.
        //asm!("movw 24(%rsp), %gs");
        //asm!("movw 16(%rsp), %fs");
        movw 8(%rsp), %es;
        movw (%rsp), %ds;
        addq $32, %rsp;
        // Pop alignment word and error.
        addq $16, %rsp;
        iretq
        "#,
        ktext_sel = const segment::KTEXT_SEL,
        cs_offset = const TRAPFRAME_CS_OFFSET,
        vector_offset = const TRAPFRAME_VECTOR_OFFSET,
        trap = sym trap,
        options(att_syntax, noreturn));
}

fn make_gate(thunk: unsafe extern "C" fn() -> !, vecnum: i32) -> segment::GateDesc {
    match vecnum {
        2 => segment::intr64(thunk, segment::IntrStack::NMI),
        3 => segment::intr64(thunk, segment::IntrStack::DB),
        8 => segment::intr64(thunk, segment::IntrStack::DFault),
        _ => segment::intr64(thunk, segment::IntrStack::RSP0),
    }
}

#[repr(C, align(4096))]
pub struct IDT {
    entries: [segment::GateDesc; 256],
}

impl IDT {
    pub const fn empty() -> IDT {
        IDT {
            entries: [segment::GateDesc::empty(); 256],
        }
    }

    pub fn init(&mut self) {
        self.entries = seq!(N in 0..=255 {
            [#(
                make_gate(vector#N, N),
            )*]
        });
    }

    pub unsafe fn load(&mut self) {
        lidt(self);
    }
}
