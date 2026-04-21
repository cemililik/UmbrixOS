//! `Cpu` and `ContextSwitch` implementations for the QEMU `virt` aarch64 target.
//!
//! `QemuVirtCpu` implements:
//! - [`umbrix_hal::Cpu`] — interrupt masking and core identity (object-safe).
//! - [`umbrix_hal::ContextSwitch`] — cooperative register-state save/restore
//!   (generic; see [ADR-0020]).
//!
//! # Safety overview
//!
//! The context-switch assembly (`context_switch_asm`) is the only intrinsically
//! unsafe operation in this file. Every other `unsafe impl` is a marker that
//! follows from the struct's invariants. See the individual `// SAFETY:` comments
//! and the audit entries `UNSAFE-2026-0006` through `UNSAFE-2026-0009`.
//!
//! [ADR-0020]: https://github.com/cemililik/UmbrixOS/blob/main/docs/decisions/0020-cpu-trait-v2-context-switch.md

use core::arch::asm;

use umbrix_hal::{ContextSwitch, CoreId, Cpu, IrqState};

// ─── QemuVirtCpu ────────────────────────────────────────────────────────────

/// The QEMU `virt` aarch64 CPU implementation.
///
/// A zero-size type — all behaviour comes from DAIF register manipulation
/// and the context-switch assembly stub. Construct via [`QemuVirtCpu::new`].
pub struct QemuVirtCpu {
    _priv: (),
}

impl QemuVirtCpu {
    /// Construct the CPU handle.
    ///
    /// # Safety
    ///
    /// There must be at most one `QemuVirtCpu` instance driving a given
    /// physical core. Creating a second instance on the same core and calling
    /// `restore_irq_state` on both may produce inconsistent DAIF state.
    /// In v1 (single-core), construct exactly once in `kernel_entry`.
    #[must_use]
    pub const fn new() -> Self {
        Self { _priv: () }
    }
}

// SAFETY: `QemuVirtCpu` is a zero-size marker; it has no interior mutability
// and holds no pointers. Sending it between threads is safe — the only shared
// hardware resource (DAIF) is accessed via per-core system registers that are
// inherently thread-local in a single-core system. Audit: UNSAFE-2026-0006.
unsafe impl Send for QemuVirtCpu {}

// SAFETY: Same reasoning as the `Send` impl — no interior mutability; DAIF
// reads/writes are atomic per-core register operations. Audit: UNSAFE-2026-0006.
unsafe impl Sync for QemuVirtCpu {}

impl Cpu for QemuVirtCpu {
    fn current_core_id(&self) -> CoreId {
        let mpidr: u64;
        // SAFETY: `MRS x, MPIDR_EL1` is a non-privileged read of a read-only
        // system register. It does not modify any state and is always available
        // in EL1. Audit: UNSAFE-2026-0007.
        unsafe {
            asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nostack, nomem));
        }
        // AFF0 (bits 7:0) identifies the core within a cluster. QEMU virt
        // presents a flat topology where AFF0 == core index.
        #[allow(clippy::cast_possible_truncation, reason = "AFF0 fits in u32")]
        let id = (mpidr & 0xFF) as u32;
        id
    }

    fn disable_irqs(&self) -> IrqState {
        let daif: usize;
        // SAFETY: `MRS x, DAIF` reads the current interrupt mask; `MSR DAIF, #0xF`
        // masks all DAIF bits (D, A, I, F). Both are EL1-privileged register
        // operations. The returned `IrqState` captures the prior value so that
        // `restore_irq_state` can restore it exactly. Audit: UNSAFE-2026-0007.
        unsafe {
            asm!(
                "mrs {daif}, daif",
                "msr daifset, #0xf",
                daif = out(reg) daif,
                options(nostack, nomem),
            );
        }
        IrqState(daif)
    }

    fn restore_irq_state(&self, state: IrqState) {
        // SAFETY: `MSR DAIF, x` writes the full DAIF register. `state` must be
        // a value previously returned by `disable_irqs`; the caller is
        // contractually bound to pass it unmodified. Writing an arbitrary value
        // could enable or suppress interrupts unexpectedly, but the contract
        // documents this requirement. Audit: UNSAFE-2026-0007.
        unsafe {
            asm!("msr daif, {}", in(reg) state.0, options(nostack, nomem));
        }
    }

    fn wait_for_interrupt(&self) {
        // SAFETY: `WFI` halts the core until an interrupt arrives. It does not
        // modify registers or memory; it only affects CPU power state.
        // Audit: UNSAFE-2026-0007.
        unsafe {
            asm!("wfi", options(nostack, nomem));
        }
    }

    fn instruction_barrier(&self) {
        // SAFETY: `ISB` synchronizes the instruction stream. It is always safe
        // to call; it cannot cause memory corruption. Audit: UNSAFE-2026-0007.
        unsafe {
            asm!("isb", options(nostack, nomem));
        }
    }
}

// ─── Aarch64TaskContext ──────────────────────────────────────────────────────

/// Saved callee-register state for one cooperative task on aarch64.
///
/// Layout must match the field offsets used by [`context_switch_asm`].
/// `#[repr(C)]` prevents field reordering.
///
/// Total size: 13 × 8 = 104 bytes per task context.
#[derive(Default)]
#[repr(C)]
pub struct Aarch64TaskContext {
    /// `x19`–`x28`: callee-saved general-purpose registers (10 × u64).
    pub x19_x28: [u64; 10],
    /// `x29` — frame pointer (callee-saved in AAPCS64).
    pub fp: u64,
    /// `x30` — link register / return address (callee-saved in AAPCS64).
    pub lr: u64,
    /// Stack pointer — saved explicitly (not a general-purpose register).
    pub sp: u64,
}

// ─── context_switch_asm ──────────────────────────────────────────────────────

/// Save `x19`–`x28`, fp, lr, sp into `*current` and restore from `*next`.
///
/// # Safety
///
/// - Both pointers must be 8-byte-aligned and valid for the duration of the
///   switch.
/// - `next` must have been written by a prior call to `context_switch_asm`
///   or fully initialised by `init_context_inner`.
/// - The caller is responsible for disabling interrupts before calling this
///   function.
///
/// Audit: UNSAFE-2026-0008.
unsafe fn context_switch_asm(current: *mut Aarch64TaskContext, next: *const Aarch64TaskContext) {
    // Field offsets within Aarch64TaskContext (repr(C)):
    //   x19_x28  offset   0  (10 × 8 = 80 bytes)
    //   fp       offset  80
    //   lr       offset  88
    //   sp       offset  96
    //
    // We save sp via `mov x2, sp` because `str sp, [x0, #96]` is not valid
    // in AArch64 — sp cannot be used as a source in most store instructions.
    // SAFETY: inline assembly that saves/restores callee-saved registers.
    // The register constraints and memory clobbers are stated explicitly.
    // Audit: UNSAFE-2026-0008.
    unsafe {
        asm!(
            // ── save current ────────────────────────────────────────────────
            "stp x19, x20, [{cur},  #0]",
            "stp x21, x22, [{cur},  #16]",
            "stp x23, x24, [{cur},  #32]",
            "stp x25, x26, [{cur},  #48]",
            "stp x27, x28, [{cur},  #64]",
            "stp x29, x30, [{cur},  #80]",   // fp, lr
            "mov x8,  sp",
            "str x8,  [{cur}, #96]",          // sp

            // ── restore next ─────────────────────────────────────────────────
            "ldr x8,  [{nxt}, #96]",          // sp
            "mov sp,  x8",
            "ldp x29, x30, [{nxt}, #80]",    // fp, lr
            "ldp x27, x28, [{nxt}, #64]",
            "ldp x25, x26, [{nxt}, #48]",
            "ldp x23, x24, [{nxt}, #32]",
            "ldp x21, x22, [{nxt}, #16]",
            "ldp x19, x20, [{nxt},  #0]",

            // ret jumps to the lr we just loaded from `next`.
            // For a task's first run, that lr was set to the entry fn by
            // init_context_inner.
            "ret",

            cur = in(reg) current,
            nxt = in(reg) next,
            // All callee-saved regs are clobbered by the restore.
            out("x8") _,
            options(nostack),
        );
    }
}

// ─── ContextSwitch impl ───────────────────────────────────────────────────────

impl ContextSwitch for QemuVirtCpu {
    type TaskContext = Aarch64TaskContext;

    unsafe fn context_switch(&self, current: &mut Self::TaskContext, next: &Self::TaskContext) {
        // SAFETY: caller guarantees interrupts are disabled and both contexts
        // are valid. We forward directly to the assembly stub which upholds
        // the AAPCS64 callee-save contract. Audit: UNSAFE-2026-0008.
        unsafe {
            context_switch_asm(
                current as *mut Aarch64TaskContext,
                next as *const Aarch64TaskContext,
            );
        }
    }

    unsafe fn init_context(
        &self,
        ctx: &mut Self::TaskContext,
        entry: fn() -> !,
        stack_top: *mut u8,
    ) {
        // Set lr to the entry function — the first `ret` in context_switch_asm
        // will jump here to begin the task.
        // Set sp to stack_top — caller must guarantee 16-byte alignment.
        // All other callee-saved registers are zero (from Default), which is
        // safe: the entry function establishes its own frame.
        // Audit: UNSAFE-2026-0009.
        ctx.lr = entry as usize as u64;
        ctx.sp = stack_top as u64;
    }
}
