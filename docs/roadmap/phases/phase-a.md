# Phase A — Kernel core on QEMU `virt`

**Exit bar:** Two kernel tasks exchange IPC messages under capability control, scheduled cooperatively, running on QEMU `virt` aarch64.

**Scope:** Single-core. MMU-off. No userspace. Everything runs in the kernel crate. The BSP is only [`bsp-qemu-virt`](../../../bsp-qemu-virt/). The goal is to get the *kernel's internal structure* right before adding the address-space / userspace / multi-core complexity that later phases layer on.

**Out of scope:** Userspace, multi-core, MMU, real hardware, network, drivers beyond the boot console.

---

## Milestone A1 — Bootable skeleton ✓ (done 2026-04-20)

Kernel boots under QEMU `virt` aarch64 and writes a greeting to the PL011 console. See [`../../architecture/boot.md`](../../architecture/boot.md) and commit `2944e7d`.

**Acceptance criteria (met):**
- `cargo kernel-build` produces a working aarch64 ELF.
- `tools/run-qemu.sh` boots the kernel under QEMU virt.
- Serial output reads `umbrix: hello from kernel_main`.

**ADRs:** ADR-0012 (boot flow).

---

## Milestone A2 — Capability table foundation (active)

Per-task capability table data structure, capability kind enum, and the in-kernel operations (`cap_copy`, `cap_derive`, `cap_revoke`, `cap_drop`) — without IPC integration. Capabilities are move-only Rust tokens; a derivation tree enforces the narrowing-only invariant and supports cascading revocation.

### Sub-breakdown

1. **ADR-0014 — Capability representation.** Data layout (struct / enum), handle type exposed to callers, rights bitfield, derivation-tree storage (intrusive vs. index-based), per-task bound on table size, error type.
2. **Module skeleton.** New `umbrix_kernel::cap` module with type definitions and doc comments, no logic yet.
3. **`cap_drop` operation** (simplest; touches only the caller's table).
4. **`cap_copy` operation** (narrowing of rights).
5. **`cap_derive` operation** (narrowing of scope; records parent-child link).
6. **`cap_revoke` operation** (subtree invalidation).
7. **Host tests.** Happy path + narrowing invariants + revocation cascade + bounded-table exhaustion.

### Acceptance criteria

- ADR-0014 Accepted before implementation code lands.
- `CapabilityTable` type with bounded, heap-free storage.
- `Capability` type (enum / struct) covering the v1 placeholder variants.
- Rights bitfield with the operations exposed so far (`Duplicate`, `Derive`, `Revoke`, `Transfer` — the last a placeholder until IPC).
- Handle-based access (`CapHandle`); raw bits never exposed.
- Four operations implemented and host-tested.
- Rights-narrowing invariant enforced (widening attempts return an error).
- Revocation cascade works for a derivation tree of depth ≥ 3.
- Table-full path returns a typed error (`CapsExhausted`), never panics or allocates.
- Ideally no new `unsafe`; any new `unsafe` gets an audit entry.

### Tasks under A2

- [T-001 — Capability table foundation](../../analysis/tasks/phase-a/T-001-capability-table-foundation.md) — Ready.
- Subsequent tasks (T-002+) will be opened as T-001 lands if further decomposition is needed; current plan is T-001 covers the milestone in one task.

### Informs

Milestone A3 cannot start until A2 is Done — kernel objects need a capability system to point through.

---

## Milestone A3 — Kernel objects

Introduce the first concrete kernel objects — `Task`, `Endpoint`, `Notification` — that capabilities point at. Scheduler and IPC still absent; this milestone is about **storage and lifecycle**.

### Sub-breakdown

1. **ADR-0015 — Kernel object storage.** Intrusive / arena / slab. Per-type vs. shared arena. Lifecycle guarantees (who owns what; when does a capability go dangling).
2. **`KernelObject` trait or enum.** A uniform way for the capability table to point at any object.
3. **`Task` kernel object.** Minimal fields — an id, a placeholder for state, a capability table reference. No scheduler interaction yet.
4. **`Endpoint` kernel object.** Fields for the IPC queues that A4 will use; structurally present but not wired up.
5. **`Notification` kernel object.** A 64-bit saturating word and a list of waiters (placeholder).
6. **Create / destroy APIs.** Allocating and freeing kernel objects under the capability discipline.
7. **Capability-to-object linkage.** The `Capability` variants replace placeholders with real kernel-object references (by handle).
8. **Host tests.** Create → capability → destroy → handle invalidation.

### Acceptance criteria

- ADR-0015 Accepted.
- Kernel-object types defined, reachable from the capability table.
- `cap_drop` of the last capability pointing at an object is observed (for reference: whether the object is freed immediately or on `destroy` is an ADR-decided detail).
- No heap; kernel objects live in a bounded pool per type.
- Host tests for lifecycle and handle invalidation pass.

### Informs

Milestone A4 builds the actual IPC paths against the `Endpoint` and `Notification` objects introduced here.

---

## Milestone A4 — IPC primitives

Synchronous rendezvous endpoints and asynchronous notifications. Capability transfer with a message is atomic with delivery.

### Sub-breakdown

1. **ADR-0016 — IPC primitive set.** Pure rendezvous vs. rendezvous + reply-recv fastpath. Blocking semantics. Message format (fixed-size vs. variable, registers vs. buffer).
2. **ADR-0017 — Badge scheme (if v1 needs it).** Per-derivation discriminator carried through to the receiver. Can be deferred if scope permits.
3. **`send` operation.** Validates the sender's `SendCap`; if a receiver waits, delivers; otherwise blocks the sender on the endpoint queue.
4. **`recv` operation.** Symmetric to `send`.
5. **`reply_recv` fastpath** (if ADR-0016 keeps it in v1).
6. **`notify` operation.** Fires a bit on a notification's saturating word; wakes any waiter.
7. **Capability transfer with message.** Moves caps atomically with delivery; validates sender holds each claimed cap.
8. **Rendezvous correctness** across sender-first and receiver-first orderings.
9. **Host tests.** Round-trip, no-receiver, capability transfer (move, not copy), notification delivery, saturation behaviour, blocked-sender wake on receive.

### Acceptance criteria

- ADR-0016 Accepted; ADR-0017 Accepted or explicitly deferred.
- `send` / `recv` / `notify` operations implemented against the A3 kernel objects.
- Capability transfer is atomic with delivery (partial-transfer failure modes ruled out by construction or by test).
- Cross-task tests (two stub "tasks" in kernel code) demonstrate the round trip.
- No new `unsafe` beyond what A3 already introduced, or each new `unsafe` is justified and audited.

### Informs

Milestone A5 needs IPC so that yield-to-peer makes sense; A6 demonstrates A4's output.

---

## Milestone A5 — Cooperative scheduler and context switch

The first real scheduler: cooperative yield-based, with a context-switch primitive that swaps register state between kernel-level "tasks." No preemption, no timer tick yet.

### Sub-breakdown

1. **ADR-0018 — Scheduler shape.** Queue structure (FIFO per priority / single queue / ring). Yield semantics ("yield to anyone" vs. "yield to a specific task"). Blocked-task handling.
2. **ADR-0019 — `Cpu` trait v2 (context-switch extension).** Adds `save_context` / `restore_context` primitives to [`umbrix-hal::Cpu`](../../../hal/src/cpu.rs). Probably adds a `TaskContext` associated type.
3. **Context-switch assembly** in `bsp-qemu-virt`: saves callee-saved regs + SP + PC, restores the target task's state.
4. **Safe Rust wrapper** for the assembly, living in the BSP with tight `unsafe` audit.
5. **Scheduler queue** — bounded, per-priority; for v1 a single FIFO is enough.
6. **`yield_now` kernel operation** — moves the current task to the back of the ready queue and switches to the head.
7. **Blocked-task state** — when a task blocks on IPC (from A4), it is removed from the ready queue until woken.
8. **Host tests** for the scheduler data structures; **QEMU smoke** for the actual context switch.

### Acceptance criteria

- ADR-0018 and ADR-0019 Accepted.
- Cpu trait v2 lands in `umbrix-hal`; BSP provides the asm.
- Two kernel-level tasks yield back and forth; this is observable via console output from inside QEMU.
- `unsafe` around the context switch is audited; the safe wrapper's invariants are stated in its `# Safety` doc.

### Informs

Milestone A6 integrates IPC + scheduling to demonstrate Phase A end-to-end.

---

## Milestone A6 — Two-task IPC demo

Integration: the kernel runs a deterministic two-task scenario where Task A sends a capability-gated message to Task B through an endpoint, B replies, and both exit cleanly.

### Sub-breakdown

1. **Demo tasks** (A and B) as kernel-level stubs (no userspace yet).
2. **QEMU smoke runner** — captures serial, asserts the expected trace (`umbrix: A sends; B receives; B replies; A receives reply; done`).
3. **Guide** — `docs/guides/two-task-demo.md` explaining what the demo proves and how to run it.
4. **Business review** — milestone A2–A6 retrospective in `docs/analysis/reviews/business-reviews/`.

### Acceptance criteria

- Deterministic QEMU trace demonstrates both tasks executed and IPC round-tripped.
- Guide committed.
- Phase A exit bar met: two kernel tasks exchange IPC messages under capability control.

### Phase A closure

When A6 is Done, run a full business review covering the whole phase. The review's output is the trigger for Phase B to become active.

---

## ADR ledger for Phase A

| ADR | Purpose | Expected state |
|-----|---------|----------------|
| ADR-0014 | Capability representation | Proposed → Accepted in A2 |
| ADR-0015 | Kernel object storage | Proposed → Accepted in A3 |
| ADR-0016 | IPC primitive set | Proposed → Accepted in A4 |
| ADR-0017 | Badge scheme (if v1 needs it) | Proposed → Accepted in A4 or explicitly deferred |
| ADR-0018 | Scheduler shape | Proposed → Accepted in A5 |
| ADR-0019 | `Cpu` trait v2 (context-switch) | Proposed → Accepted in A5 |

Numbers may shift if unexpected decisions land in between; sequencing here is intent, not reservation.

## Open questions carried into Phase A

- Whether A4's IPC primitive set needs badges in v1 (ADR-0017) or can defer them.
- Whether A5's scheduler needs priority classes in v1 (ADR-0018 decides; preference is single class for now).
- Whether `Cpu` v2 stays one trait with new methods or spawns a sibling `ContextSwitch` trait (ADR-0019 decides).
