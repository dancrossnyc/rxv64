use crate::arch;
use crate::proc::{self, myproc};
use crate::sysfile;
use crate::trap::ticks;

pub unsafe fn init() {
    const MSR_STAR: u32 = 0xc000_0081;
    const MSR_LSTAR: u32 = 0xc000_0082;
    const MSR_FMASK: u32 = 0xc000_0084;
    arch::wrmsr(MSR_LSTAR, enter as usize as u64);
    arch::wrmsr(MSR_STAR, arch::star());
    arch::wrmsr(MSR_FMASK, arch::sfmask());
}

extern "C" fn syscall(a0: usize, a1: usize, a2: usize, num: usize) -> i64 {
    use syslib::syscall::*;
    match num {
        FORK => myproc().fork().map(i64::from).unwrap_or(-1),
        EXIT => myproc().exit(),
        WAIT => myproc().wait().map(i64::from).unwrap_or(-1),
        PIPE => -1,
        READ => {
            if let Ok(r) = sysfile::read(a0, a1, a2) {
                r as i64
            } else {
                -1
            }
        }
        KILL => proc::kill(a0 as u32).map_or(-1, |_| 0),
        EXEC => {
            crate::println!("Exec!");
            -1
        }
        FSTAT => {
            if let Ok(()) = sysfile::stat(a0, a1) {
                0
            } else {
                -1
            }
        }
        CHDIR => -1,
        DUP => -1,
        GETPID => i64::from(myproc().pid()),
        SBRK => {
            if let Ok(sz) = myproc().adjsize(a0 as isize) {
                sz as i64
            } else {
                -1
            }
        }
        SLEEP => -1,
        UPTIME => ticks() as i64,
        OPEN => -1,
        WRITE => {
            if let Ok(r) = sysfile::write(a0, a1, a2) {
                r as i64
            } else {
                -1
            }
        }
        MKNOD => -1,
        UNLINK => -1,
        LINK => -1,
        MKDIR => -1,
        CLOSE => {
            if let Some(file) = myproc().free_fd(a0) {
                file.close();
                0
            } else {
                -1
            }
        }
        _ => {
            crate::println!("syscall number {}, a1={}, a2={}, a3={}", num, a0, a1, a2);
            -1
        }
    }
}

#[naked]
unsafe extern "C" fn enter() -> ! {
    // Switch user and kernel GSBASE
    asm!(r#"
        swapgs

        // Stash the user stack pointer and set the kernel
        // stack pointer.  Use %r8 as a scratch register,
        // since it is callee-save and we clear on return
        // anyway.
        movq %rsp, %r8
        movq %gs:16, %rsp

        // Save callee-saved registers, flags and the stack pointer.
        // This is a `struct Context` at the top of the kernel stack.
        // If we know that we came into the kernel via a system call,
        // we can use this to retrieve the Context structure.  We use
        // this in e.g. fork() to copy state from the parent to the child.
        pushq %rcx
        pushq %r15
        pushq %r14
        pushq %r13
        pushq %r12
        pushq %rbx
        pushq %rbp

        // Save user %rflags
        pushq %r11

        // User stack frame.
        pushq %r8

        // Set up a call frame so that we can get a back trace
        // from here, possibly into user code
        pushq %rcx
        pushq %rbp
        movq %rsp, %rbp

        // System call number is 4th argument to `syscall` function.
        movq %rax, %rcx

        // Call the handler in Rust.
        sti
        callq {syscall}
        cli

        // Pop activation record from the stack.
        addq $16, %rsp
        jmp {syscallret}
        "#,
        syscall = sym syscall,
        syscallret = sym syscallret,
        options(att_syntax, noreturn)
    );
}

#[naked]
pub unsafe extern "C" fn syscallret() {
    // Pop context copy of user stack pointer.  Uses r8
    // as a scratch register: we clear it anyway.
    asm!(
        r#"
        pop %r8;

        // Restore callee-saved registers.
        popq %r11;
        popq %rbp;
        popq %rbx;
        popq %r12;
        popq %r13;
        popq %r14;
        popq %r15;
        popq %rcx;

        // Save kernel stack pointer in per-CPU structure.
        movq %rsp, %gs:16;

        // Restore user stack pointer.
        movq %r8, %rsp;

        // Clear caller-saved registers to avoid leaking kernel
        // data into userspace.
        xorq %rdi, %rdi;
        xorq %rsi, %rsi;
        xorq %rdx, %rdx;
        xorq %r10, %r10;
        xorq %r9, %r9;
        xorq %r8, %r8;

        // Switch kernel, user GSBASE
        swapgs;

        // Return from system call
        sysretq;
        "#,
        options(att_syntax, noreturn)
    );
}
