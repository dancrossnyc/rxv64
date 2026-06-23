use core::arch::global_asm;
use core::slice;
use syslib::syscall::{EXEC, EXIT};

global_asm!(
    r#"
.align 16
.globl start_init, start_init_len
start_init:
	// exec(init, argv);
	movq	${EXEC}, %rax
	movq	$(init - start_init), %rdi
	movq	$(argv - start_init), %rsi
	syscall

	// exit() in a loop.
1:	movq	${EXIT}, %rax
	syscall
	jmp	1b

.align 8
init: .string "/init\0"
.align 8
argv:
	.quad init - start_init;
	.quad 0;

start_init_len: .quad . - start_init
    "#,
    EXEC = const EXEC,
    EXIT = const EXIT,
    options(att_syntax)
);

unsafe extern "C" {
    fn start_init() -> !;
    static start_init_len: usize;
}

pub fn start_init_slice() -> &'static [u8] {
    let start_ptr = start_init as *const ();
    let start = start_ptr.addr();
    let len = unsafe { start_init_len };
    assert!(len < 200);
    unsafe { slice::from_raw_parts(start as *const u8, len) }
}
