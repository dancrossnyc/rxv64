ENTRY(entry)

SECTIONS {
	. = 0xFFFF800000100000;

	.text . :
	{
		*(.text.boot)
		*(.text*)
		. = ALIGN(4096);
		*(.trap)
	}

	. = ALIGN(4096);

	PROVIDE(etext = .);

	.rodata . :
	{
		*(.rodata*)
	}

	. = ALIGN(4096);

	PROVIDE(erodata = .);

	.data . :
	{
		*(.data*)
	}

	. = ALIGN(4096);

	PROVIDE(edata = .);

	.bss . :
	{
		*(.bss* COMMON)
	}

	. = ALIGN(4096);

	PROVIDE(end = .);
}
