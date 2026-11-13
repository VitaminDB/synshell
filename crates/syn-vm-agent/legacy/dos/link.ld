/* .COM: всё в одном сегменте с 0x100 (CS = DS = ES = SS), стек — у конца сегмента. */
OUTPUT_FORMAT("elf32-i386")
ENTRY(_start)
SECTIONS {
    . = 0x100;
    .text : { *(.text.entry) *(.text .text.*) }
    .rodata : { *(.rodata .rodata.*) }
    .data : { *(.data .data.*) }
    .bss : { __bss_start = .; *(.bss .bss.*) *(COMMON) __bss_end = .; }
    /DISCARD/ : { *(.eh_frame*) *(.comment) *(.note*) *(.got*) }
}
ASSERT(. <= 0xE800, "SYNAGENT.COM больше 58 КиБ — стеку не хватит места")
