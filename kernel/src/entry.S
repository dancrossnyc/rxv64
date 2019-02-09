// The rv64 kernel starts executing in this file, which is linked with
// the kernel Rust code so that it can refer to kernel symbols such as
// main().  The boot block (bootasm.S and bootmain.c) jumps to `start`
// below.

// Multiboot header, for multiboot boot loaders like GNU Grub.
// http://www.gnu.org/software/grub/manual/multiboot/multiboot.html
//
// Using GRUB 2, you can boot xv6 from a file stored in a
// Linux file system by copying kernel or kernelmemfs to /boot
// and then adding this menu entry:
//
// menuentry "xv6" {{
//       insmod ext2
//       set root='(hd0,msdos1)'
//       set kernel='/boot/kernel'
//       echo "Loading ${{kernel}}..."
//       multiboot ${{kernel}} ${{kernel}}
//       boot
// }}

.set STACKSIZE,		4096
.set PAGESIZE,		4096

.set CR0_PE,		1 << 0
.set CR0_PG,		1 << 31
.set CR0_BOOT64,	CR0_PE + CR0_PG

.set CR4_PSE,		1 << 4
.set CR4_PAE,		1 << 5
.set CR4_FSGSBASE,	1 << 16
.set CR4_BOOT,		CR4_PAE + CR4_PSE + CR4_FSGSBASE

.set IA32_MSR_EFER,	0xc0000080
.set EFER_SCE,		1 << 0
.set EFER_LME,		1 << 8
.set EFER_NX,		1 << 11
.set EFER_BOOT_LO,	EFER_SCE + EFER_LME + EFER_NX
.set EFER_BOOT_HI,	0x00000000

.set GDT_NULL,		0
.set GDT_CODE64,	1 << 3
.set GDT32_CODE,	2 << 3
.set GDT32_DATA,	3 << 3

.set SEG_READ,		1 << 41
.set SEG_WRITE,		1 << 42
.set SEG_CODE,		1 << 43
.set SEG_DATA,		0 << 43
.set SEG_ALWAYS1,	1 << 44
.set SEG_PRESENT,	1 << 47
.set SEG_LONG,		1 << 53

.set SEG32_DEFAULT,	1 << 54
.set SEG32_GRANULARITY,	1 << 55
.set SEG32_LIMIT,	(0xF << 48) + 0xFFFF
.set SEG32_DEF,		SEG32_DEFAULT + SEG32_GRANULARITY + SEG32_LIMIT

.set KERNBASE,		0xFFFF800000000000

.set MULTIBOOT1_MAGIC,	0x1BADB002
.set MULTIBOOT1_FLAGS,	1 << 1	// Provide memory map.

.section .text.boot
.align 8
.globl multiboot1_header
multiboot1_header:
.int	MULTIBOOT1_MAGIC
.int	MULTIBOOT1_FLAGS
.int	-(MULTIBOOT1_MAGIC + MULTIBOOT1_FLAGS)

.code32
.globl entry
entry:
	cli
	cld

	movl	$(bootstack - KERNBASE + STACKSIZE), %esp

	movl	$IA32_MSR_EFER, %ecx
	movl	$EFER_BOOT_LO, %eax
	movl	$EFER_BOOT_HI, %edx
	wrmsr

	movl	%cr4, %eax
	orl	$CR4_BOOT, %eax
	movl	%eax, %cr4

	movl	$(entrypgtbl - KERNBASE), %eax
	movl	%eax, %cr3

	movl	$CR0_BOOT64, %eax
	movl	%eax, %cr0
	nop
	nop

	lgdt	(entrygdtdesc - KERNBASE)
	ljmpl	$GDT_CODE64, $(entry64 - KERNBASE)

.code64
entry64:
	xorw	%ax, %ax
	movw	%ax, %ds
	movw	%ax, %es
	movw	%ax, %fs
	movw	%ax, %gs
	movw	%ax, %ss
	movabsq	$(bootstack + STACKSIZE), %rsp
	movabsq	$1f, %rax
	pushq	%rax
	ret
1:
	// Call `main` in Rust code.
	movq	%rbx, %rdi
	pushq	$0
	movq	%rsp, %rbp
	callq	main
1:	cli
	hlt
	jmp	1b

.align 4096
.rodata

.set APENTRY,		0x7000
.set APPERCPU,		0x8000 - 24
.set APID,		0x8000 - 16
.set APSEMAPHORE,	0x8000 - 8

// While it may seem like this should be in a text section,
// it is deliberately not.  AP entry code is explicitly copied
// to a page in low memory for execution, so as far as the rest
// of the kernel is concerned it's simply read-only data.  We
// put it into .rodata so that it's mapped onto a non-executable
// page and the kernel cannot accidentally jump into it once it
// is running in Rust code on a real page table.
.globl apentry, eapentry
.code16
.align 4096
apentry:
	cli
	cld
	wbinvd

	lgdtl	(APENTRY + (apgdtdesc - apentry))

	movl	%cr0, %eax
	orl	$CR0_PE, %eax
	movl	%eax, %cr0

	ljmpl   $GDT32_CODE, $(APENTRY + (apentry32 - apentry))

.code32
apentry32:
	movw    $GDT32_DATA, %ax
	movw    %ax, %ds
	movw    %ax, %ss
	movw    %ax, %es
	xorw    %ax, %ax
	movw    %ax, %fs
	movw    %ax, %gs

	movl	$IA32_MSR_EFER, %ecx
	movl	$EFER_BOOT_LO, %eax
	movl	$EFER_BOOT_HI, %edx
	wrmsr

	movl	%cr4, %eax
	orl	$CR4_BOOT, %eax
	movl	%eax, %cr4

	movl	$(entrypgtbl - KERNBASE), %eax
	movl	%eax, %cr3

	movl	$CR0_BOOT64, %eax
	movl	%eax, %cr0
	nop
	nop

	ljmp	$GDT_CODE64, $(APENTRY + (apentry64 - apentry))

.code64
apentry64:
	xorw	%ax, %ax
	movw	%ax, %ds
	movw	%ax, %es
	movw	%ax, %fs
	movw	%ax, %gs
	movw	%ax, %ss

	movq	APPERCPU, %rdi
	movq	APID, %rsi
	movq	APSEMAPHORE, %rdx

	movq	%rdi, %rsp
	addq	$4096, %rsp
	movabsq	$1f, %rax
	pushq	%rax
	ret
1:

	pushq	$0
	movq	%rsp, %rbp
	call	mpenter
	ret

1:	cli
	hlt
	jmp	1b

.align 16

gdt:
// 0: Null segment
.quad	0
// 8: Kernel 64-bit code segment
.quad	(SEG_READ + SEG_CODE + SEG_ALWAYS1 + SEG_PRESENT + SEG_LONG)
// 16: Kernel 32-bit code segment (for bootstrapping APs)
.quad	(SEG_READ + SEG_CODE + SEG_ALWAYS1 + SEG_PRESENT + SEG32_DEF)
// 24: Kernel 32-bit data segment (for bootstrapping APs)
.quad	(SEG_READ + SEG_WRITE + SEG_ALWAYS1 + SEG_PRESENT + SEG32_DEF)
egdt:

.skip 6

apgdtdesc:
.word	egdt - gdt - 1
.long	(APENTRY + gdt - apentry)

eapentry:

.align 4096
.globl entrypgtbl
entrypgtbl:
	.quad (entrypgtbl3 - KERNBASE) + (1<<1) + (1<<0)
	.space (4096/2) - (1*8)
	.quad (entrypgtbl3 - KERNBASE) + (1<<1) + (1<<0)
	.space (4096/2) - (1*8)

.align 4096
entrypgtbl3:
	.quad (entrypgtbl2 - KERNBASE) + (1<<1) + (1<<0)
	.space 4096 - 1*8

.align 4096
entrypgtbl2:
	.quad (0<<20) + (0<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (2<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (4<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (6<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (8<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (10<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (12<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (14<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (16<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (18<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (20<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (22<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (24<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (26<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (28<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (30<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (32<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (34<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (36<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (38<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (40<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (42<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (44<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (46<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (48<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (50<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (52<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (54<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (56<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (58<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (60<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (62<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (64<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (66<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (68<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (70<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (72<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (74<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (76<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (78<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (80<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (82<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (84<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (86<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (88<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (90<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (92<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (94<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (96<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (98<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (100<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (102<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (104<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (106<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (108<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (110<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (112<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (114<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (116<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (118<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (120<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (122<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (124<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.quad (126<<20) + (1<<63) + (1<<7) + (1<<1) + (1<<0)
	.space 4096 - 64*8

.align 16
.skip 6
entrygdtdesc:
.word	egdt - gdt - 1
.long	(gdt - KERNBASE)

.bss
.align 4096

// Stack.
.globl bootstack
.comm	bootstack, STACKSIZE
