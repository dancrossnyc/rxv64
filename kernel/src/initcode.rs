use core::concat;
use core::slice;
use syslib::syscall::{EXEC, EXIT};

#[naked]
unsafe extern "C" fn start_init() -> ! {
    asm!(concat!(
        "1: .align 16;",

        // exec(init, argv);
        r#"
        movq ${EXEC}, %rax;
        movq $(init - 1b), %rdi;
        movq $(argv - 1b), %rsi;
        syscall;"#,

        // loop { exec(init, argv); }
        r#"
        2: movq ${EXIT}, %rax;
        syscall;
        jmp 2b;

        .align 8
        init: .string "/init\0";
        .align 8
        argv:
            .quad init - 1b;
            .quad 0;

        .globl start_init_len;
        start_init_len:
            .quad . - 1b
        "#),
        EXEC = const EXEC,
        EXIT = const EXIT,
        options(att_syntax, noreturn)
    );
}

extern "C" {
    static start_init_len: usize;
}

pub fn start_init_slice() -> &'static [u8] {
    let start = start_init as usize;
    let len = unsafe { start_init_len };
    assert!(len < 200);
    unsafe { slice::from_raw_parts(start as *const u8, len) }
}
