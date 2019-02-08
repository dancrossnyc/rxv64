// Low level support for ulib.

// System call stubs.  Note that the KBI ("kernel binary interface")
// uses almost the same calling convention as the ABI ("application
// binary interface").  So system calls are actually pretty
// straight-forward.  We generate "naked" functions in a macro.
// An improvement would be to specify the function signatures
// here for calling Rust code.

#[cfg(not(test))]
mod syscalls {
    macro_rules! sysasm {
        ($r:ident, $num:expr) => {
            asm!("syscall" : "={rax}"($r)
                : "{rax}"($num as usize)
                : "rcx","r11" : "volatile");
        };
        ($r:ident, $num:expr, $a:ident) => {
            asm!("syscall" : "={rax}"($r)
                : "{rax}"($num as usize), "{rdi}"($a as u64)
                : "rcx","r11" : "volatile");
        };
        ($r:ident, $num:expr, $a:ident, $b:ident) => {
            asm!("syscall" : "={rax}"($r)
                : "{rax}"($num as usize), "{rdi}"($a as u64), "{rsi}"($b as u64)
                : "rcx","r11" : "volatile");
        };
        ($r:ident, $num:expr, $a:ident, $b:ident, $c:ident) => {
            asm!("syscall" : "={rax}"($r)
                : "{rax}"($num as usize),
                  "{rdi}"($a as u64),
                  "{rsi}"($b as u64),
                  "{rdx}"($c as u64)
                : "rcx","r11" : "volatile");
        };
    }
    macro_rules! syscall {
        // , $( $a:ident : $at:ty),* => $name($($a:$at),*)
        ($name:ident, $num:expr, $rety:ty $(, $a:ident : $at:ty)*) => {
            #[no_mangle]
            #[naked]
            pub unsafe extern "C" fn $name($($a : $at,)*) -> $rety {
                let r: $rety;
                sysasm!(r, $num $(,$a)*);
                r
            }
        };
    }
    use syslib::syscall as SYS;

    syscall!(fork, SYS::FORK, i32);
    syscall!(exit, SYS::EXIT, i32);
    syscall!(wait, SYS::WAIT, i32);
    syscall!(pipe, SYS::PIPE, i32, fds: *mut i32);
    syscall!(write, SYS::WRITE, i32, fd: i32, buf: *const u8, n: usize);
    syscall!(read, SYS::READ, i32, fd: i32, buf: *mut u8, n: usize);
    syscall!(close, SYS::CLOSE, i32, fd: i32);
    syscall!(kill, SYS::KILL, i32, pid: i32);
    syscall!(
        exec,
        SYS::EXEC,
        i32,
        path: *const u8,
        args: *const *const u8
    );
    syscall!(open, SYS::OPEN, i32, path: *const u8, mode: i32);
    syscall!(
        mknod,
        SYS::MKNOD,
        i32,
        path: *const u8,
        major: i16,
        minor: i16
    );
    syscall!(unlink, SYS::UNLINK, i32, path: *const u8);
    syscall!(fstat, SYS::FSTAT, i32, sb: *mut u8);
    syscall!(link, SYS::LINK, i32, a: *const u8, b: *const u8);
    syscall!(mkdir, SYS::MKDIR, i32, path: *const u8);
    syscall!(chdir, SYS::CHDIR, i32, path: *const u8);
    syscall!(dup, SYS::DUP, i32, fd: i32);
    syscall!(getpid, SYS::GETPID, i32);
    syscall!(sbrk, SYS::SBRK, *mut u8, incr: i32);
    syscall!(sleep, SYS::SLEEP, i32, ticks: i32);
    syscall!(uptime, SYS::UPTIME, i32);
}

// Note: the very existence of this block of code annoys me.
// The System V x86_64 ABI specification is very precise in
// describing how variadic functions, as in C, implement
// argument passing.  Since there is a register-based calling
// convention on this architecture, retrieving arguments is
// not as simple as just indirecting a pointer into the
// caller's stack frame and va_list is not just a pointer.
//
// Instead, va_list is a multi-word structure for
// describing overflow and register save areas and offsets
// for both general purpose and floating point registers.
// The details of constructing that structure are processor
// specific and creating the structure is complicated.
//
// The key observation is that the function that creates the
// va_list must be able to precisely introspect its stack
// frame, as it must (potentially) reach into _its_ callers
// frame to get arguments that overflow the available general
// purpose registers.  As a consequence, creating the va_list
// structure all but requires compiler support instead of
// being a simple `va_start` macro.  Thus compilers like
// Clang and GCC provide intrinsics for this that are
// automatically lowered into variadic functions.  While not
// simple at least the interface is easy to use from the
// programmer's perspective.
//
// However, at the time of writing those intrinsics are not
// fully exposed in the Rust compiler.  Rust can consume a
// va_list, but doesn't (yet) support creating one.  So while
// we can write `vprintf` directly in Rust, we cannot (yet)
// write variadic functions like `printf`.
//
// Instead of resorting to a C stub, we just write `printf`
// here in assembly language.  The heavy lifting is still done
// by Rust code, but we have to set up the va_list manually.

global_asm!(
    r#"
// `printf` exists only to construct a `va_list` on the stack
// and pass it `rvprintf`.  The layout of `va_list` is defined
// precisely by the ABI, so this code should be "stable" with
// respect to compiler changes, etc.  Still, we'd rather write
// this in Rust.  At some point support for variadic functions
// will be added to Rust and we then should take the opportunity
// to replace this with a proper Rust function.
//
// According to the System V ABI specification, x86_64 processor
// supplement, the exact format the of va_list structure is:
//
// typedef struct {
// 	unsigned int gp_offset;
// 	unsigned int fp_offset;
// 	void *overflow_arg_area;
// 	void *reg_save_area;
// } va_list[1];
//
// Note that `unsigned int` is a 32-bit quantity here, and `void *`
// is 64-bit.  The ABI guarantees that there is no padding in this
// structure and that it will be naturally aligned to 8 bytes.
//
// We provide two implementations of `printf` here.  One is
// called `slow_printf`: this is more straight forward in order
// to illustrate exactly what goes where using push's etc, but
// the side effects of operations on the stack throw etc make
// it more difficult for the processor to speculate and perform
// ad hoc parallelism.
//
// The second implementation largely copies what a compiler would
// generate.

.globl printf

// The stack layout here is:
//
// | caller stack....             |
// /                              /
// | caller stack argument(s)...  | 16 <-- "overflow arguments" in va_list
// +------------------------------+
// | return address               |
// | saved frame pointer          |   0 <-- %rbp points here
// +------------------------------+
// | saved %r9                    | -8,
// | saved %r8                    | -16
// | saved %rcx                   | -24
// | saved %rdx                   | -32
// | empty (first arg in %rdi)    | -40 <-- Note GP offset is 16 skipping these...
// | empty (second arg in %rsi)   | -48 <-- ...two args.  %rsp points here
// | va_list4: reg save area ptr  | -56 <-- Points at offset %rbp -48
// | va_list3: overflow area ptr  | -64 <-- Points to caller stack arguments
// | va_list2: fp  +--------------+ -68 <-- 32-bit FP offset (=48, in bytes)
// | va_list1: gp  ^--------------+ -72 <-- 32-bit GP offset (=16, in bytes)
// | Skip a word for alignment    | -80   <-- %rsp points here.
// +------------------------------+
printf:
slow_printf:
	pushq	%rbp		// Save caller frame pointer
	movq	%rsp, %rbp	// Procedure linkage

	// Save argument registers into the register save area.
	pushq	%r9
	pushq	%r8
	pushq	%rcx
	pushq	%rdx
	pushq	$0		// Dummy value for %rsi
	pushq	$0		// Dummy value for %rdi

	// Copy the address of the saved reg area into the va_list
	pushq	%rsp
	// Copy the address of the overflow area into the va_list
	lea	10(%rbp), %rax
	push	%rax

	// Copy the GP and FP register offsets into the va_list.
	// Note that the GP offset is in the lower 32-bit
	// half of the register, and FP offset in the upper half.
	movq	$(((6*8)<<32)+(2*8)), %rax
	pushq	%rax		// Copy offsets into va_list struct

	// Set pointer to va_list as 3rd argument to rvprintf.
	// Note that the first two arguments are already in the
	// correct registers: %rdi holds the FD, %rsi the format
	// string pointer.
	movq	%rsp, %rdx

	pushq	$0		// Align the stack

	// Do the actual printing.
	xorl	%eax, %eax
	callq	rvprintf

	// Return.
	movq	%rbp, %rsp
	popq	%rbp
	retq

// This is the "fast" version of `printf`.  The stack layout
// is as follows.  Note that the va_list is laid out so that
// it is aligned on a 16-byte boundary, presumably so that it
// could be copied using two 16-byte aligned mov's.
//
// | caller stack                 |
// /                              /
// | caller stack argument(s)...  | 16
// +------------------------------+
// | return address               |
// | saved frame pointer          |   0 <-- %rbp points here
// +------------------------------+
// | Skip a word for alignment    |  -8
// | va_list4: reg save area ptr  | -16 <-- Points at offset %rbp - 80
// | va_list3: overflow area ptr  | -24 <-- Points to caller stack arguments.
// | va_list2: fp  |                -28 <-- 32-bit FP offset (=48, in bytes)
// | va_list1: gp  |                -32 <-- 32-bit GP offset (=16, in bytes)
// | saved %r9                    | -40
// | saved %r8                    | -48
// | saved %rcx                   | -54
// | saved %rdx                   | -64
// | empty (first arg in %rdi)    | -72 <-- Note GP offset is 16 skipping these...
// | empty (second arg in %rsi)   | -80 <-- ...two args.  %rsp points here.
// +------------------------------+   ^----  %rsp points here.
fast_printf:
	push	%rbp		// Save caller frame pointer
	movq	%rsp, %rbp	// Set new frame pointer

	subq	$80, %rsp	// Allocate stack space

	// Save arguments passed in general purpose registers.
	// Saves are all relative to the frame pointers, which
	// points to the top of the stack frame.
	//
	// Note that the first two arguments to rvprintf will
	// be passed in registers, so the bottom two words
	// of the register save area are ignored.
	movq	%r9, -40(%rbp)	// 6th argument register
	movq	%r8, -48(%rbp)	// 5th argument register
	movq	%rcx, -56(%rbp)	// 4th argument register
	movq	%rdx, -64(%rbp)	// 3rd argument register

	// Copy address reg save area into va_list
	movq	%rsp, -16(%rbp)

	// Copy address of overflow args area into va_list
	lea	16(%rbp), %rax
	movq	%rax, -24(%rbp)

	movl	$(6*8), -28(%rbp)
	movl	$(2*8), -32(%rbp)

	// Note that the first two arguments to rvprintf (the
	// file descriptor and pointer to the format string) are
	// already in %rdi and %rsi, respectively.  We load the
	// address of the va_list structure into %edx as the
	// third argument.
	lea	-32(%rbp), %rdx	// 3rd arg == va_list pointer.

	// Call `rvprintf` to do the actual printing.
	xorl	%eax, %eax	// Clear %rax
	callq	rvprintf	// Perform the actual printing.

	// Clean up the stack and return.  Note the `add`
	// instruction to reset the stack pointer: why do we use
	// this instead of `movq %rbp, %rsp`?  Because `%rbp`
	// will presumably be modified by a pop from the stack
	// in `rvprintf`.  `addq $80, %rsp` is just adding an
	// immediate to a register; in the dynamic instruction
	// stream seen by the processor, this can presumably be
	// done while the read from the stack to `%rbp` in the
	// calling function is still completing.  The only thing
	// `rvprintf` (presumably) does to `%rsp` is add
	// constants to it, so there's no similar penalty.
	addq	$80, %rsp	// Pop everything off the stack
	popq	%rbp		// Restore caller frame pointer
	retq
"#
);
