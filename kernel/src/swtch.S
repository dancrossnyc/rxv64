// Context switch
//
//   pub unsafe fn swtch(from: *mut *const Context, to: *const Context)
//
// Save the current registers on the stack, creating
// a Context struct, and save its address in `*from`.
// Switch stacks to `to` and pop previously-saved registers.

.text

.globl swtch
swtch:
	// Save callee-saved registers.
	// RIP is already on the stack.
	pushq	%r15
	pushq	%r14
	pushq	%r13
	pushq	%r12
	pushq	%rbx
	pushq	%rbp
	pushfq
	pushq	%rsp

	// Switch stacks
	movq	%rsp, (%rdi)
	movq	%rsi, %rsp

	// Restore previously saved callee-saved registers
	addq	$16, %rsp
	//popfq
	popq	%rbp
	popq	%rbx
	popq	%r12
	popq	%r13
	popq	%r14
	popq	%r15

	// Return to previous location
	retq
