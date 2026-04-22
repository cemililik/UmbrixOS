# T-007 — Idle task + typed `SchedError::Deadlock` + resume-path hardening

- **Phase:** B
- **Milestone:** B0 — Phase A exit hygiene
- **Status:** In Progress
- **Created:** 2026-04-22
- **Author:** @cemililik (+ Claude Opus 4.7 agent)
- **Dependencies:** [T-006](T-006-raw-pointer-scheduler-api.md) (`In Review`, ADR-0021 Accepted — the raw-pointer bridge shape this task's idle-registration path inherits).
- **Informs:** [T-011](.) (Missing-tests bundle — adds the direct unit test for `SchedError::Deadlock` once this task lands). Unblocks every B1+ milestone that depends on a panic-free scheduler.
- **ADRs required:** [ADR-0022 — Idle task and typed scheduler deadlock error](../../../decisions/0022-idle-task-and-typed-scheduler-deadlock.md) *(Accepted 2026-04-22)*.

---

## User story

As the Umbrix kernel scheduler, I want a registered idle task and typed error variants for the two current kernel-liveness panics — so that a userspace-caused IPC graph with no runnable task cannot panic the kernel, and the resume-path `debug_assert!` in `ipc_recv_and_yield` becomes a release-mode typed return instead of a silent fall-through that the caller decodes as a panic one frame up.

## Context

The A5 scheduler shipped with three hard-panic paths documented in [ADR-0022](../../../decisions/0022-idle-task-and-typed-scheduler-deadlock.md):

1. `sched::ipc_recv_and_yield` panics with `"deadlock: all tasks blocked on IPC and no idle task available"` when every task is blocked and the ready queue is empty.
2. `sched::start` panics when called without any registered task.
3. A `debug_assert!(!matches!(result, Ok(RecvOutcome::Pending)))` in `ipc_recv_and_yield`'s resume path is stripped in release builds, allowing a `RecvOutcome::Pending` to propagate to the caller — where the BSP's `let RecvOutcome::Received { … } = outcome else { panic!(…) }` turns it into a downstream panic.

The 2026-04-21 Phase-A-exit security review flagged path (1) as §4 (liveness); the code review flagged all three. ADR-0022 resolves the design: idle-as-regular-task (Option A) + two new error surfaces (Option G: `SchedError::Deadlock` and `IpcError::PendingAfterResume`). Path (2)'s `start` empty-queue panic is **kept** — it is a boot-time programming error, not a runtime condition.

T-007 lands the implementation: register an idle task during `kernel_entry`, extend `SchedError` and `IpcError`, and wire the typed returns through `ipc_recv_and_yield`. Because ADR-0022 chose "idle as regular task", there is no new scheduler API — idle uses `sched::add_task` + `sched::yield_now` unchanged.

## Acceptance criteria

- [x] **ADR-0022 Accepted** (2026-04-22) — idle-as-regular-task + typed `SchedError::Deadlock` + `IpcError::PendingAfterResume`; `start`'s empty-queue panic kept.
- [ ] **`SchedError::Deadlock` variant added** to `kernel::sched::SchedError` and returned by `ipc_recv_and_yield` in place of the current `panic!("deadlock: …")`.
- [ ] **`IpcError::PendingAfterResume` variant added** to `kernel::ipc::IpcError` and returned through `SchedError::Ipc(…)` in place of the current release-mode fall-through on the `debug_assert!`.
- [ ] **BSP idle task registered.** `bsp-qemu-virt/src/main.rs` adds a `fn idle_entry() -> !` that loops `cpu.wait_for_interrupt()` + `sched::yield_now(...)`, a `TASK_IDLE_STACK: TaskStack`, and a `create_task` + `add_task` for idle inside `kernel_entry` *before* `sched::start`. Idle is added first so it is never mistakenly the first-dispatched task (task B still runs first per the existing A6 ordering).
- [ ] **Tests:** a kernel unit test directly constructs a scheduler, blocks the sole task via `ipc_recv_and_yield` without registering an idle task, and asserts `Err(SchedError::Deadlock)`. A second test covers `IpcError::PendingAfterResume` by simulating a resume where the endpoint state is `Pending` (test-only scaffold — may use a `FakeCpu` that returns instead of switching).
- [ ] **Tests stay green.** 75 kernel + 34 test-hal = 109 host tests. QEMU smoke reproduces the A6 five-line trace exactly (idle must not print; its presence must be invisible to the demo).
- [ ] **No new `unsafe`.** ADR-0022 §Consequences/Positive guarantees idle introduces no new `unsafe` pattern; this criterion is the explicit check.
- [ ] **Documentation:** `ipc_recv_and_yield`'s `# Errors` section documents `SchedError::Deadlock` and `SchedError::Ipc(IpcError::PendingAfterResume)`; `IpcError` doc lists `PendingAfterResume`'s semantics; the scheduler module doc mentions that the BSP registers an idle task.

## Out of scope

- Idle task priority / per-priority queues — priorities are not on the B0/B1 roadmap (ADR-0019); idle-as-lowest-priority is a follow-up ADR if priorities land later.
- Kernel-owned idle entry function (Option D in ADR-0022) — explicitly rejected; BSPs own their idle entry.
- Timer IRQ wiring that would let idle wake up on tick — T-009.
- `const { assert!(N > 0) }` on `SchedQueue::new` / `CapabilityTable::new` — routed to T-010's hardening bundle unless it falls out naturally from this work.
- `TASK_ARENA_CAPACITY` bump — not needed; 16 minus idle = 15 is plenty for v1.
- Any code outside `kernel/src/sched/mod.rs`, `kernel/src/ipc/mod.rs`, `bsp-qemu-virt/src/main.rs`, and the audit log / docs it touches.

## Approach

Settled in ADR-0022 §Decision outcome. At sketch level, in commit order:

1. **Extend `IpcError` with `PendingAfterResume`.** One-line variant addition plus doc.
2. **Extend `SchedError` with `Deadlock`.** One-line variant addition plus doc; keep the existing `Ipc(IpcError)` variant — `PendingAfterResume` propagates through it.
3. **Rewire `ipc_recv_and_yield`:**
   - Replace the `panic!("deadlock: …")` in the Phase 2 block with `return Err(SchedError::Deadlock)` — and restore `s.current` + `s.task_states[current_idx]` to their pre-block state before returning so the caller is not left in an inconsistent scheduler state.
   - In Phase 3 (resume), replace the `debug_assert!` + `result.map_err(SchedError::Ipc)` pattern with an explicit match that converts `Ok(RecvOutcome::Pending)` into `Err(SchedError::Ipc(IpcError::PendingAfterResume))`. Keep the `debug_assert!` as a test-mode complement so the invariant is still loudly violated in debug builds.
4. **BSP idle registration.**
   - Add `static TASK_IDLE_STACK: TaskStack = TaskStack::new();` next to the existing stacks.
   - Add `fn idle_entry() -> !` that loops `cpu.wait_for_interrupt()` then `sched::yield_now(SCHED.as_mut_ptr(), cpu)`; `expect`s the `yield_now` Result since it can only error with `NoCurrentTask` (impossible once scheduler has started).
   - In `kernel_entry`: `create_task` for idle (before A and B so its handle is lowest-numbered), then `add_task` on the scheduler *before* the existing `add_task(B)` and `add_task(A)` — this keeps B as the first-dispatched task, matching the A6 trace.
5. **Tests.**
   - `sched::tests::deadlock_returns_err_when_no_idle` — builds a `Scheduler<FakeCpu>`, adds one task, blocks it on an endpoint via the bridge, asserts `SchedError::Deadlock`. Requires a test-only path through `ipc_recv_and_yield` that exercises the empty-ready-queue branch with `FakeCpu::context_switch` never actually being called.
   - `sched::tests::pending_after_resume_returns_err` — constructs the scheduler, drives a resume path where the endpoint state is forced to stay `Pending`, asserts `SchedError::Ipc(IpcError::PendingAfterResume)`.
   - Both tests must not require a real QEMU boot — they run in `cargo host-test`.
6. **QEMU smoke.** Boot under QEMU, capture the five-line trace, confirm byte-for-byte match with the A6 baseline.
7. **Task status → In Review**, `current.md` updated, commit sequence matches the projected list in §Design notes.

## Definition of done

- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo host-clippy` clean with `-D warnings`.
- [ ] `cargo kernel-clippy` clean.
- [ ] `cargo host-test` passes (at least 77 kernel + 34 test-hal = 111 host tests — +2 from the new scheduler tests).
- [ ] `cargo kernel-build` clean; QEMU smoke reproduces the A6 trace unchanged (idle is invisible).
- [ ] ADR-0022 Accepted before the implementation commit (2026-04-22, commit `7fb74bb` + this task's opening commit).
- [ ] No new `unsafe` blocks introduced — no audit-log changes required.
- [ ] Commit messages follow [`commit-style.md`](../../../standards/commit-style.md) with `Refs: ADR-0022` trailers.
- [ ] Task status updated to `In Review`; [`docs/roadmap/current.md`](../../../roadmap/current.md) updated.

## Design notes

- **Why restore state before returning `Err(SchedError::Deadlock)`?** The current `panic!` path leaves `s.current = None` and `s.task_states[current_idx] = Blocked { on: ep_handle }` because it never returns. A typed return that *doesn't* restore these leaves the caller's scheduler view inconsistent with the task's actual state (the task is still running — it's the one that just observed the `Err`). The restore block is small (one line for `current`, one for `task_states`), and it keeps the bridge's invariants composable: after `Err(Deadlock)` the scheduler state is exactly what it was before the bridge was called. The test asserts this directly.
- **Why two variants across two enums instead of one?** ADR-0022 §Option G weighs this. Short form: scheduler faults live in `SchedError`; IPC faults live in `IpcError`. `PendingAfterResume` is semantically an IPC invariant ("sender failed to deliver before unblocking receiver"); it propagates through `SchedError::Ipc(…)` exactly like `IpcError::InvalidCapability` does today.
- **Why is `start`'s empty-queue panic kept?** Boot-time programming error. The only way to reach it is `kernel_entry` that forgot to `add_task`. Converting to `Err` means `kernel_entry` has to panic one frame up instead — strictly more code and less informative. Panicking where the invariant is violated is correct.
- **Idle registration order matters.** `add_task(idle)` must come before `add_task(B)` and `add_task(A)` so idle is in slot 0 of the ready queue but gets dequeued last. Actually the opposite — FIFO means the first-added runs first. So idle must be added **last** so B still runs first. Re-reading the approach: adding idle *after* A/B ensures B is dequeued first. Confirm on implementation; the test `yield_now_switches_context_and_updates_current` style test in sched/mod.rs is the reference for order.
- **Idle-yields-to-itself path.** When idle is the only ready task, its `yield_now` call observes the "only one ready task" early-return (`sched/mod.rs` line 385-389-ish in the current `yield_now`). The idle loop therefore degrades to a tight WFI loop — `wfi` → `yield_now` returns immediately → `wfi` again — with no context switch. Matches ADR-0022 §Option A's efficiency claim.
- **Projected commit sequence.**
  1. `docs(roadmap): open T-007 — idle task + typed scheduler deadlock (B0)` (this opening commit — task file + current.md + phase-b/README.md + phase-b.md + ADR-0022 accept).
  2. `feat(sched,ipc): SchedError::Deadlock + IpcError::PendingAfterResume; wire typed returns (T-007)`.
  3. `feat(bsp): register idle task in kernel_entry (T-007)`.
  4. `test(sched): deadlock and pending-after-resume return Err (T-007)`.
  5. `docs(roadmap): T-007 → In Review`.

## References

- [ADR-0022 — Idle task and typed scheduler deadlock error](../../../decisions/0022-idle-task-and-typed-scheduler-deadlock.md) — the design this task implements.
- [ADR-0019 — Scheduler shape](../../../decisions/0019-scheduler-shape.md) — defines `SchedError`.
- [ADR-0017 — IPC primitive set](../../../decisions/0017-ipc-primitive-set.md) — defines `IpcError`.
- [ADR-0021 — Raw-pointer scheduler IPC-bridge API](../../../decisions/0021-raw-pointer-scheduler-ipc-bridge.md) — the bridge shape idle's `add_task` + idle_entry's `yield_now` calls inherit.
- [Security review — Umbrix → Phase A exit](../../reviews/security-reviews/2026-04-21-umbrix-to-phase-a.md) — §4 flags the deadlock panic.
- [Code review — Umbrix → Phase A exit](../../reviews/code-reviews/2026-04-21-umbrix-to-phase-a.md) — *Correctness (Scheduler bullets 2, 4)* flags all three panics.
- [T-006 mini-retro](../../reviews/business-reviews/2026-04-22-T-006-mini-retro.md) — "post-In-Review second-read" adjustment: schedule a preventative-assert pass at `In Review` before promoting to `Done`.
- [Phase B plan](../../../roadmap/phases/phase-b.md) — §B0 item 2 bundles this work.

## Review history

| Date | Reviewer | Note |
|------|----------|------|
| 2026-04-22 | @cemililik (+ Claude Opus 4.7 agent) | opened; status `In Progress`. ADR-0022 Accepted earlier today (commit `7fb74bb`). Implementation cleared to begin: `IpcError::PendingAfterResume` + `SchedError::Deadlock` variants → `ipc_recv_and_yield` rewire → BSP idle registration → two new host tests. Current.md pointed at T-007; T-006 moves off `Active task`, remains `In Review` pending maintainer promotion. |
