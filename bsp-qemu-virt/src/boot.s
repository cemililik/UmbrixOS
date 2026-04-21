/*
 * QEMU virt aarch64 reset entry.
 *
 * See docs/decisions/0012-boot-flow-qemu-virt.md and
 * docs/architecture/boot.md for the design.
 *
 * Responsibilities, in order:
 *   1. Set the stack pointer to __stack_top (from linker.ld).
 *   2. Enable FP/SIMD at EL1: set CPACR_EL1.FPEN = 0b11 so that the
 *      compiler-generated NEON instructions (e.g. movi/stp q-regs for
 *      zero-initialisation) do not trap as Undefined Instruction.
 *   3. Zero the BSS region [__bss_start, __bss_end), which is 8-byte
 *      aligned at both ends so 8-byte stores are safe.
 *   4. Branch to kernel_entry (a Rust function marked extern "C").
 *   5. If kernel_entry ever returns (it should not), halt defensively.
 *
 * QEMU virt drops the kernel to EL1 before execution. The DTB pointer
 * in x0 is currently ignored. No EL transition performed here
 * (per ADR-0012 v1).
 */

    .section .text.boot, "ax"
    .global _start

_start:
    adrp    x0, __stack_top
    add     x0, x0, :lo12:__stack_top
    mov     sp, x0

    /* Enable FP/SIMD at EL1 and EL0 (CPACR_EL1.FPEN = 0b11 = bits[21:20]).
     * 0x300000 = 3 << 20.  CPACR_EL1 resets to zero, so only FPEN needs
     * setting; all other fields remain 0 (no ZEN, no TTA traps).
     * ISB ensures the write is visible before the first NEON instruction. */
    mov     x0, #0x300000
    msr     cpacr_el1, x0
    isb

    adrp    x0, __bss_start
    add     x0, x0, :lo12:__bss_start
    adrp    x1, __bss_end
    add     x1, x1, :lo12:__bss_end
0:
    cmp     x0, x1
    b.hs    1f
    str     xzr, [x0], #8
    b       0b

1:
    bl      kernel_entry

2:
    wfe
    b       2b
