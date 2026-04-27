/*
 * Linker script for tyrne-bsp-qemu-virt.
 *
 * See docs/decisions/0012-boot-flow-qemu-virt.md for the memory layout
 * rationale. In short: QEMU virt loads us at 0x40080000 via -kernel;
 * _start must be first in .text; BSS is 8-byte aligned so the BSS-zero
 * loop can use 8-byte stores; 64 KiB stack is reserved after .bss and
 * named __stack_top at its high end.
 */

ENTRY(_start)

MEMORY {
    RAM (rwx) : ORIGIN = 0x40080000, LENGTH = 128M
}

SECTIONS {
    . = ORIGIN(RAM);

    .text : {
        KEEP(*(.text.boot))
        /* aarch64 EL1 exception vector table — VBAR_EL1 requires
         * 2 KiB (2048-byte) alignment per ARM ARM §D11.2. The
         * `.text.vectors` section holds the 16-entry table assembled
         * in `src/vectors.s`; KEEP prevents linker GC. */
        . = ALIGN(2048);
        KEEP(*(.text.vectors))
        *(.text .text.*)
    } > RAM

    .rodata : ALIGN(8) {
        *(.rodata .rodata.*)
    } > RAM

    .data : ALIGN(8) {
        *(.data .data.*)
    } > RAM

    .bss : ALIGN(8) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(8);
        __bss_end = .;
    } > RAM

    . = ALIGN(16);
    . = . + 64K;
    __stack_top = .;

    /DISCARD/ : {
        *(.comment)
        *(.note.*)
        *(.eh_frame*)
        *(.gcc_except_table*)
    }
}
