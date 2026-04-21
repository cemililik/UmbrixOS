# 0021 — Raw-pointer scheduler IPC-bridge API

- **Status:** Proposed
- **Date:** 2026-04-22
- **Deciders:** @cemililik

## Context

[`UNSAFE-2026-0012`](../audits/unsafe-log.md) records a `&mut` aliasing hazard in the cooperative scheduler's IPC bridge. The bodies of `task_a` and `task_b` in [`bsp-qemu-virt/src/main.rs`](../../bsp-qemu-virt/src/main.rs) materialise `&mut` references to `SCHED`, `EP_ARENA`, `IPC_QUEUES`, and the per-task `CapabilityTable`, and hold those references live across the `cpu.context_switch` call inside [`Scheduler::ipc_send_and_yield`](../../kernel/src/sched/mod.rs) / [`Scheduler::ipc_recv_and_yield`](../../kernel/src/sched/mod.rs). When the *other* task resumes, it acquires its own `&mut` references to the same `UnsafeCell` interiors. Under Rust's strict aliasing model two live `&mut` references to the same referent is immediately undefined behaviour — the compiler is entitled to optimise as if each were uniquely aliased, regardless of whether the accesses occur simultaneously.

Umbrix v1's single-core cooperative execution model happens to shield the current compiled binary from observable miscompilation: no two tasks ever execute at once, and the `naked_asm!` context-switch barrier is opaque enough that today's LLVM does not rearrange loads or stores around it. But this is a compile-time accident, not a guarantee. A future LLVM version, a change in the inliner's budgets, or the introduction of preemption, SMP, or the MMU could all break the shield without code changes on our side. The 2026-04-21 [security review of Phase A exit](../analysis/reviews/security-reviews/2026-04-21-umbrix-to-phase-a.md) makes this the **#1 Phase-B blocker**, to be closed before any later Phase B milestone (EL drop, MMU activation, per-task address spaces, syscall boundary, userspace) compounds the hazard.

This ADR chooses the API shape that retires UNSAFE-2026-0012. It is the architectural decision that governs [`T-006`](../analysis/tasks/phase-b/T-006-raw-pointer-scheduler-api.md); implementation and audit-log updates follow as task work.

## Decision drivers

- **Rust aliasing model correctness.** The primary driver. A bridge API that requires the caller to hold a `&mut` across `cpu.context_switch` is unsound under the language contract; any solution must make that pattern unreachable from the BSP task bodies.
- **Minimum invasive change.** Phase A shipped with the current API; 75 kernel-crate tests exercise it; 34 test-hal tests back it. We are fixing a specific hazard, not redesigning the scheduler. A refactor that ripples through unrelated subsystems owes the project a justification it does not need today.
- **Forward compatibility with preemption and SMP.** The chosen shape must remain sound when the cooperative-cover invariant (no two tasks execute simultaneously) no longer holds. A preemption-capable scheduler wraps shared state in per-CPU locks; whatever API we pick must adapt to that.
- **Consistency with existing `Scheduler` internals.** [`Scheduler::yield_now`](../../kernel/src/sched/mod.rs#L310-L316) already uses a split-borrow via raw-pointer arithmetic on `self.contexts`, audited under UNSAFE-2026-0008. Any new aliasing discipline should extend this precedent rather than invent a separate idiom.
- **Zero-`unsafe` kernel crate preservation.** A3–A6 closed with every `unsafe` block in the BSP or in scheduler code where the unsafety was intrinsic to the context-switch call. New `unsafe` should not leak into `kernel::cap`, `kernel::obj`, or `kernel::ipc`.
- **BSP ergonomics.** Task bodies must stay auditable. A per-call nested closure or a type-erased continuation is more expensive to read than a direct sequence of `unsafe` blocks, and readability at the `unsafe` boundary is a security property, not a taste preference.
- **Testability.** The 11 existing scheduler tests construct `FakeCpu` + `AlignedStack` + stack-local arenas and drive the bridge directly. Whatever shape we pick must not force those tests to route through a new ownership layer.
- **Audit surface.** Each new `unsafe` must carry a `// SAFETY:` comment conforming to [`unsafe-policy.md §1`](../standards/unsafe-policy.md#1-every-unsafe-use-is-justified-in-a--safety-comment) and a corresponding `UNSAFE-2026-NNNN` entry in the audit log.

## Considered options

1. **Option A — Raw-pointer IPC-bridge parameters.** `Scheduler::ipc_send_and_yield` / `Scheduler::ipc_recv_and_yield` accept `*mut EndpointArena`, `*mut IpcQueues`, and `*mut CapabilityTable` instead of `&mut`. Callers (the BSP task bodies) construct pointers via `UnsafeCell::get()` without first materialising a `&mut`. Inside the scheduler, each pointer is momentarily dereferenced to a `&mut` only for the duration of the pre-switch work; the reference is dropped strictly before `cpu.context_switch` runs. The `&mut self` borrow that persists across the switch is a single intra-struct borrow the scheduler itself owns — a sound split-borrow, not an external aliasing hazard.
2. **Option B — Scheduler owns the shared arenas.** `Scheduler<C>` grows to own `EndpointArena`, `IpcQueues`, and the per-task `CapabilityTable`s. The IPC bridge takes no external state parameters; callers hand the scheduler a `TaskHandle` and a `CapHandle` and receive a result. The BSP hands ownership of every arena to the scheduler at bootstrap and never touches them again.
3. **Option C — Continuation-passing bridge.** Callers pass a closure describing the resume path; the scheduler suspends the caller, performs the switch, and on resume invokes the closure, which re-acquires fresh references inside its own scope. No `&mut` crosses the switch at the API level because nothing the compiler sees persists across it.
4. **Option D — Per-task `TaskContext` extensions.** Each task's `TaskContext` is extended to carry references (or raw pointers) to its own arenas. The scheduler accesses these per-task during dispatch, never as parameters to an IPC call — effectively a variant of Option B where the ownership is per-task rather than global.

## Decision outcome

**Chosen: Option A — Raw-pointer IPC-bridge parameters.**

Option A retires UNSAFE-2026-0012 with the smallest disturbance to the Phase A codebase that works. The `Scheduler<C>` struct keeps its current shape (ready queue + per-task state + saved contexts); `ipc_send` / `ipc_recv` / `cap_take` / `insert_root` keep their current signatures; the 11 scheduler tests recompile unchanged because `FakeCpu`-driven tests already thread stack-local arenas into the bridge and can pass raw pointers to them just as easily as `&mut`. The new `unsafe` is narrow and localised: one audit entry in the BSP for the `UnsafeCell::get()` acquisition helper, one audit entry inside the scheduler for the momentary-borrow pattern that replaces direct `&mut` parameter use. UNSAFE-2026-0012 retires.

The decision also preserves `Scheduler::yield_now`'s existing raw-pointer idiom on `self.contexts`. The new IPC-bridge `unsafe` blocks will look like existing ones — the reader who has already audited UNSAFE-2026-0008 sees the same shape, reuses the same mental model, and the review cost is approximately flat. This matters more than it might seem: an inconsistent `unsafe` idiom raises the review cost of every change that touches the scheduler, and the scheduler is on the critical path for the rest of Phase B.

Option B is the second-best alternative. It is rejected because `Scheduler<C>` would grow to own `EndpointArena`, `IpcQueues`, and every per-task `CapabilityTable` — pulling three separate subsystems' state into one struct that today orchestrates them at arm's length. The current layering is deliberate: `kernel::cap`, `kernel::obj`, and `kernel::ipc` are independently importable, independently testable, and each covers its own ADR (0014, 0016, 0017). Merging their state into the scheduler would couple their lifecycles to scheduler bring-up and tear-down, force every IPC-layer host test to construct a dummy scheduler, and break `ipc_*` unit tests that today run against raw `IpcQueues` / `EndpointArena`. Worse, Option B fights preemption: per-CPU locking on Phase C's shared state wants per-arena granularity, not a single scheduler-held lock on a god-object.

Option C is rejected because the closure solves the wrong problem: its captured `&mut` references are exactly the aliasing hazard we are trying to eliminate. Forcing captures to be by value or by raw pointer defeats the ergonomics argument that motivated the option in the first place. Additionally, closure erasure (`Box<dyn FnOnce>`) requires an allocator we do not have, and static dispatch (`fn ipc_send<F: FnOnce(…)>(f: F)`) multiplies the kernel's monomorphised code size for no measured benefit.

Option D is rejected because it relocates rather than eliminates the hazard. Adding references inside `TaskContext` means the scheduler, when iterating `task_handles[]` or dispatching, has to materialise `&mut`s to those per-task references — and those `&mut`s can cross the switch just as the external ones do today. It also violates the ADR-0020 contract that `TaskContext` is a register-save frame, not a Rust-level state bag.

## Consequences

### Positive

- **UNSAFE-2026-0012 retires.** Its status becomes `Removed — <T-006 commit SHA>`. The BSP's task bodies no longer hold `&mut` references to shared kernel state across `cpu.context_switch`. The one remaining aliasing-adjacent window is `Scheduler`'s own `&mut self`, which is a single intra-struct borrow; `&mut self` across a method boundary is sound by construction and has no language-level concern.
- **Forward-compatible with preemption and SMP.** Raw pointers are trivially wrappable by per-CPU locks: `(*CELL.0.get()).as_mut_ptr()` becomes `LOCK.lock().arenas_mut().as_mut_ptr()` when Phase C lands. `&mut` parameters, by contrast, force the lock to be held across the entire bridge call — which blocks the target task's reacquisition of the same lock during its own IPC.
- **Consistency with `Scheduler::yield_now`.** The same raw-pointer split-borrow idiom that already runs under UNSAFE-2026-0008 now covers the IPC bridge. One review model covers both.
- **Test harness unaffected.** 75 kernel + 34 test-hal host tests continue to pass without structural changes; the pointer arguments are constructed trivially from stack-local arenas in tests.
- **Scheduler interface remains layered.** `kernel::cap`, `kernel::obj`, and `kernel::ipc` stay independently importable. Future ADRs can evolve each subsystem without scheduler churn.

### Negative

- **IPC-bridge signatures are less safe-Rust-native.** Accepting `*mut X` instead of `&mut X` loses the compile-time non-aliasing guarantee on the function parameters. *Mitigation:* the aliasing contract is stated in doc-comments on each pointer parameter, the scheduler module doc cites this ADR, every caller's `// SAFETY:` block references this ADR, and `cargo miri test` is added to the scheduler test suite so the host-level aliasing checks run in CI (once CI exists; K3-7). In practice the two in-tree callers (`task_a`, `task_b`) are the only ones that will ever exist at the BSP layer; the userspace task model introduced in Phase B's syscall boundary routes through a kernel dispatcher that calls the scheduler on behalf of the task, so the userspace-reachable attack surface of this API does not grow.
- **New `unsafe` surface in the BSP and the scheduler.** The BSP needs a helper to turn `&StaticCell<T>` into `*mut T` without materialising a `&mut`; the scheduler needs an internal helper that performs the momentary `*mut → &mut` deref. *Mitigation:* two new audit entries (one per helper), each pointing to a single documented SAFETY comment the helper carries once, each used from one-line call sites that cite the helper's audit tag. This is a strict improvement over the current state: today four separate `// SAFETY (aliasing)` blocks in the BSP describe the same hazard; tomorrow two audit entries describe two narrow helpers with clear preconditions.
- **Documentation debt.** The scheduler gains a paragraph in its module doc-comment describing the aliasing contract. The future architecture doc [`docs/architecture/scheduler.md`](../architecture/scheduler.md) (T-008) formalises this. *Mitigation:* T-008 is already planned in B0; the ADR explicitly names the doc location.

### Neutral

- **`TaskArena` migration is bundled with T-006 implementation.** phase-b.md §B0 couples K3-11 (`TaskArena` local → global `StaticCell`) with this refactor because both touch the same BSP static-cell surface. The bundling is a task-level decision, not an ADR-level one; the API shape this ADR decides is independent of how `TaskArena` is stored.
- **`Scheduler<C>` struct layout is unchanged.** All state the scheduler currently holds stays where it is. The change is API-shape only.
- **`ContextSwitch` / `Cpu` v2 contracts (ADR-0020) are unchanged.** The context-switch primitive itself retains its `&mut TaskContext` / `&TaskContext` signature; that borrow lives inside `Scheduler` and does not touch external arenas.

## Pros and cons of the options

### Option A — Raw-pointer bridge parameters (chosen)

- **Pro:** Smallest possible change to retire UNSAFE-2026-0012.
- **Pro:** Extends `Scheduler::yield_now`'s existing audited split-borrow pattern rather than inventing a new idiom.
- **Pro:** Forward-compatible with preemption and SMP — raw pointers wrap cleanly in locks; `&mut` parameters do not.
- **Pro:** Audit surface is narrow: one helper + one audit entry per location (BSP + scheduler), UNSAFE-2026-0012 retires.
- **Pro:** Test harness is unaffected.
- **Con:** Loses the compile-time non-aliasing guarantee on function parameters; the invariant becomes a doc-comment contract verified by review and `cargo miri test` on host tests.
- **Con:** Each momentary `*mut → &mut` deref inside the scheduler needs a `SAFETY:` comment (mitigated via the private helper pattern described in *Consequences — Negative*).

### Option B — Scheduler owns the shared arenas

- **Pro:** Most "safe Rust" feeling; zero `&mut` crosses the switch at the API level because no external state is passed in.
- **Pro:** No raw pointers anywhere in the bridge.
- **Con:** Invasive — `Scheduler<C>` becomes a god-object owning `EndpointArena`, `IpcQueues`, and per-task `CapabilityTable`s; the current deliberate layering across `kernel::cap` / `kernel::obj` / `kernel::ipc` / `kernel::sched` collapses.
- **Con:** Breaks 11 scheduler host tests that drive the bridge with stack-local arenas; every test would need a `Scheduler::new_with_arenas(...)` factory.
- **Con:** Breaks IPC-layer host tests that exercise `ipc_send` / `ipc_recv` directly against raw `IpcQueues` / `EndpointArena`; those tests would need to route through a dummy scheduler.
- **Con:** Conflicts with preemption and SMP — per-CPU locks on arenas want per-arena granularity; a single scheduler-held lock serialises every IPC across every CPU.
- **Con:** Ownership of per-task `CapabilityTable`s by the scheduler is architecturally wrong — the table is a property of the task, not the scheduler.

### Option C — Continuation-passing bridge

- **Pro:** In principle most flexible; the closure can hold any state the caller wants.
- **Pro:** Natural basis for a future async extension (Phase G+).
- **Con:** Closure-captured references still cross `cpu.context_switch` — the captured `&mut`s are exactly the aliasing hazard we are eliminating. Forcing captures to be by value or by raw pointer defeats the ergonomics argument.
- **Con:** Closure erasure either requires an allocator (`Box<dyn FnOnce>`) — incompatible with `no_std` + no-heap — or pays generic-instantiation code size (`fn<F: FnOnce()>`), which bloats the kernel binary without measured benefit.
- **Con:** Unfamiliar bare-metal idiom; higher review cost per change.
- **Con:** Panic / failure handling across the closure boundary is harder to reason about than direct error returns.

### Option D — Per-task `TaskContext` extensions

- **Pro:** No bridge parameters at all — each task's per-task state is carried internally.
- **Con:** Relocates the aliasing hazard rather than eliminating it. The scheduler, iterating `task_handles[]`, would materialise `&mut`s to per-task references that can still live across the switch.
- **Con:** Violates ADR-0020's `TaskContext` contract — `Aarch64TaskContext` is a register-save frame (`x19`–`x28`, `fp`, `lr`, `sp`, `d8`–`d15`), not a Rust-level state bag. Extending it with references changes its purpose.
- **Con:** Makes `ContextSwitch::init_context` significantly more complex; the `fn() -> !` entry-function contract no longer suffices.

## References

- [ADR-0013 — Roadmap and planning process](0013-roadmap-and-planning.md) — the planning framework inside which this ADR is written.
- [ADR-0014 — Capability representation](0014-capability-representation.md) — defines `CapabilityTable`, one of the arenas the bridge API takes as a pointer.
- [ADR-0016 — Kernel object storage](0016-kernel-object-storage.md) — defines `EndpointArena`; motivates the companion `TaskArena` migration bundled with T-006.
- [ADR-0017 — IPC primitive set](0017-ipc-primitive-set.md) — defines `IpcQueues` and the rendezvous semantics the bridge preserves.
- [ADR-0019 — Scheduler shape](0019-scheduler-shape.md) — the scheduler shape this ADR preserves.
- [ADR-0020 — `ContextSwitch` trait and `Cpu` v2](0020-cpu-trait-v2-context-switch.md) — the context-switch abstraction around which the aliasing window opens.
- [UNSAFE-2026-0012 — audit entry](../audits/unsafe-log.md) — the hazard this ADR closes.
- [Security review of Phase A exit](../analysis/reviews/security-reviews/2026-04-21-umbrix-to-phase-a.md) — §1 and §3, where the blocker is enumerated.
- [Code review of Phase A exit](../analysis/reviews/code-reviews/2026-04-21-umbrix-to-phase-a.md) — §Correctness (Scheduler bullet 2) flags the same surface.
- [Phase B plan §B0](../roadmap/phases/phase-b.md) — the milestone this ADR opens.
- [T-006 — Raw-pointer scheduler API refactor](../analysis/tasks/phase-b/T-006-raw-pointer-scheduler-api.md) — the implementing task.
- [`unsafe-policy.md`](../standards/unsafe-policy.md) — the discipline every new `unsafe` block upholds.
- [`Scheduler::yield_now`'s split-borrow pattern](../../kernel/src/sched/mod.rs#L310-L316) — the in-tree precedent.
- Rust reference, ["Behaviour considered undefined"](https://doc.rust-lang.org/reference/behavior-considered-undefined.html) — specifies that two live `&mut` references to the same referent is UB regardless of access timing.
- [`miri` — the interpreter that catches aliasing UB in host tests](https://github.com/rust-lang/miri) — the verification path proposed in *Consequences — Negative*.
- seL4 kernel call structure — each task's TCB holds a capability-slot pointer and receives IPC through the TCB without cross-task aliasing; prior art for the ownership direction Option B proposed.
