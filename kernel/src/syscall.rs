use crate::arch;
use crate::println;
use crate::proc::{self, myproc};
use crate::sysfile;
use crate::trap;
use core::arch::naked_asm;
use core::fmt::Debug;

pub unsafe fn init() {
    const MSR_STAR: u32 = 0xc000_0081;
    const MSR_LSTAR: u32 = 0xc000_0082;
    const MSR_FMASK: u32 = 0xc000_0084;
    let enter = enter as *const ();
    unsafe {
        arch::wrmsr(MSR_LSTAR, enter.addr() as u64);
        arch::wrmsr(MSR_STAR, arch::star());
        arch::wrmsr(MSR_FMASK, arch::sfmask());
    }
}

fn to_i64<T: TryInto<i64> + Debug>(v: T) -> i64
where
    <T as TryInto<i64>>::Error: Debug,
{
    v.try_into().unwrap()
}

extern "C" fn syscall(a0: usize, a1: usize, a2: usize, num: usize) -> i64 {
    use syslib::syscall::*;
    let proc = myproc();
    let r = match num {
        FORK => proc.fork().map_or(-1, i64::from),
        EXIT => proc.exit(a0 as i32),
        WAIT => proc.wait(a0).map_or(-1, i64::from),
        PIPE => sysfile::pipe(proc, a0).map_or(-1, |_| 0),
        READ => sysfile::read(proc, a0, a1, a2).map_or(-1, to_i64),
        KILL => proc::kill(a0 as u32).map_or(-1, |_| 0),
        EXEC => sysfile::exec(proc, a0, a1).map_or(-1, |_| 0),
        FSTAT => sysfile::stat(proc, a0, a1).map_or(-1, |_| 0),
        CHDIR => sysfile::chdir(proc, a0).map_or(-1, |_| 0),
        DUP => sysfile::dup(proc, a0).map_or(-1, to_i64),
        GETPID => i64::from(proc.pid()),
        SBRK => proc.adjsize(a0 as isize).map_or(-1, to_i64),
        SLEEP => trap::ticksleep(proc, a0 as u64).map_or(-1, |_| 0),
        UPTIME => trap::ticks() as i64,
        OPEN => sysfile::open(proc, a0, a1).map_or(-1, to_i64),
        WRITE => sysfile::write(proc, a0, a1, a2).map_or(-1, to_i64),
        MKNOD => sysfile::mknod(proc, a0, a1 as u32, a2 as u32).map_or(-1, |_| 0),
        UNLINK => sysfile::unlink(proc, a0).map_or(-1, |_| 0),
        LINK => sysfile::link(proc, a0, a1).map_or(-1, |_| 0),
        MKDIR => sysfile::mkdir(proc, a0).map_or(-1, |_| 0),
        CLOSE => sysfile::close(proc, a0).map_or(-1, |_| 0),
        _ => {
            println!("syscall number {num}, a0={a0}, a1={a1}, a2={a2}");
            -1
        }
    };
    if proc.dead() {
        proc.exit(1);
    }
    r
}

#[unsafe(naked)]
unsafe extern "C" fn enter() -> ! {
    // Switch user and kernel GSBASE
    naked_asm!(r#"
        swapgs

        // Stash the user stack pointer and set the kernel
        // stack pointer.  Use %r8 as a scratch register,
        // since it is callee-save and we clear on return
        // anyway.
        movq %rsp, %r8
        movq %gs:16, %rsp

        // We construct a trap frame on the stack, but many of the
        // fields therein are not used by the system call machinery.
        // We push them anyway.
        //
        // Save callee-saved registers, flags and the stack pointer.
        // This is a `struct Context` at the top of the kernel stack.
        // If we know that we came into the kernel via a system call,
        // we can use this to retrieve the Context structure.  We use
        // this in e.g. fork() to copy state from the parent to the child.
        pushq $0    // %ss
        pushq %r8   // user stack pointer
        pushq %r11  // user %rflags

        movq %cs, %r11
        pushq %r11  // user %cs

        pushq %rcx  // user %rip

        pushq $0    // error
        pushq $0    // vector

        pushq $0    // user %gs
        movw %gs, (%rsp)
        pushq $0    // user %fs
        movw %fs, (%rsp)
        pushq $0    // user %es
        movw %es, (%rsp)
        pushq $0    // user %ds
        movw %ds, (%rsp)

        pushq %r15
        pushq %r14
        pushq %r13
        pushq %r12
        pushq $0    // %r11 was trashed
        pushq $0    // %10 is caller-save
        pushq $0    // %r9 is caller-save
        pushq $0    // %r8 is caller-save (and used as scratch)
        pushq %rbp
        pushq $0    // %rdi is caller-save
        pushq $0    // %rsi is caller-save
        pushq $0    // %rdx is caller-save
        pushq $0    // %rcx was trashed
        pushq %rbx
        pushq %rax

        // Push a dummy word to align the stack.
        pushq $0

        // Set up a call frame so that we can get a back trace
        // from here, possibly into user code.
        pushq %rcx
        movq %r11, %rbp

        // System call number is 4th argument to `syscall` function.
        movq %rax, %rcx

        // Call the handler in Rust.
        // XXX: Could we `sti` here?
        callq {syscall}

        // Pop stack frame and dummy word.
        addq $(8 * 2), %rsp
        jmp {syscallret}
        "#,
        syscall = sym syscall,
        syscallret = sym syscallret,
        options(att_syntax)
    );
}

#[unsafe(naked)]
pub unsafe extern "C" fn syscallret() {
    naked_asm!(
        r#"
        cli
        // Skip %rax. It is the return value from the system call.
        addq $8, %rsp

        popq %rbx
        // skip %rcx; it is handled specially.
        addq $8, %rsp
        popq %rdx
        popq %rsi
        popq %rdi
        popq %rbp
        popq %r8
        popq %r9
        popq %r10
        popq %r11
        popq %r12
        popq %r13
        popq %r14
        popq %r15

        // Restore user segmentation registers.
        movw (%rsp), %ds
        movw 8(%rsp), %es
        movw 16(%rsp), %fs
        // %gs is specially restored by `swapgs`, below.
        addq $(8 * 4), %rsp

        // Skip vector and error.
        addq $(8 * 2), %rsp

        // user %rip goes into %rcx
        popq %rcx

        // skip %cs
        addq $8, %rsp

        // user flags go in %r11
        popq %r11

        // copy user stack pointer into %r8
        popq %r8

        // Skip %ss
        addq $8, %rsp

        // Save kernel stack pointer in per-CPU structure.
        movq %rsp, %gs:16

        // Restore user stack pointer.
        movq %r8, %rsp
        xorq %r8, %r8

        // Switch kernel, user GSBASE
        swapgs

        // Return from system call
        sysretq
        "#,
        options(att_syntax)
    );
}
