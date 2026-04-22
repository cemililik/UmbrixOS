# Host-test coverage baseline — 2026-04-23

Captured with `cargo llvm-cov --workspace --exclude tyrne-bsp-qemu-virt` on commit `c25f8c8`. This is Tyrne's first recorded coverage measurement; it establishes a baseline against which every subsequent measurement is compared.

Raw lcov output: `docs/analysis/reports/lcov-2026-04-23.info` (git-ignored; regenerate with `cargo llvm-cov --workspace --exclude tyrne-bsp-qemu-virt --lcov --output-path docs/analysis/reports/lcov-2026-04-23.info`).

## Why BSP is excluded

`tyrne-bsp-qemu-virt` is a `no_std` / `no_main` bare-metal binary whose panic handler conflicts with `std`'s `panic_impl` lang item when built for the host target. BSP code is exercised indirectly by the QEMU smoke test; automated coverage would require the same kind of tooling a CI pipeline does (build for `aarch64-unknown-none`, run QEMU, collect `-C instrument-coverage` profiles). Routed to T-009 follow-up.

## Headline numbers

| Scope | Regions | Functions | Lines |
|---|---|---|---|
| **Workspace (excl. BSP)** | **94.41 %** | **90.37 %** | **92.85 %** |

Strong for a pre-alpha OS kernel. Interpretation: the code we *can* test on the host is densely exercised; the remaining gaps are concentrated in a small number of specific places, not diffused noise.

## Per-file, sorted weakest to strongest

| File | Regions | Functions | Lines | Notes |
|------|---------|-----------|-------|-------|
| `hal/src/mmu.rs` | 40.82 % | 35.71 % | 38.64 % | Trait declaration surface; no production impl yet (MMU lands in B2). Gap is expected and routed to B2. |
| `kernel/src/sched/mod.rs` | 84.16 % | 75.00 % | 82.80 % | **The real gap.** Detailed triage below. |
| `kernel/src/cap/mod.rs` | 89.47 % | 100.00 % | 88.89 % | Small; 2 uncovered regions at 85-86. |
| `kernel/src/obj/endpoint.rs` | 93.75 % | 88.89 % | 92.68 % | One `get_endpoint` no-op helper unused. |
| `kernel/src/obj/notification.rs` | 96.33 % | 91.67 % | 95.38 % | One unused getter. |
| `kernel/src/cap/table.rs` | 96.84 % | 98.15 % | 95.18 % | 40 uncovered regions across the file — worth a targeted sweep. |
| `kernel/src/ipc/mod.rs` | 97.73 % | 100.00 % | 97.66 % | Error-path branches; fine. |
| `kernel/src/obj/arena.rs` | 97.25 % | 100.00 % | 99.17 % | One edge case. |
| `kernel/src/cap/rights.rs` | 97.50 % | 94.44 % | 96.00 % | Bit-flag rarely exercised. |
| `kernel/src/obj/task.rs` | 100.00 % | 100.00 % | 100.00 % | — |
| `hal/src/console.rs` | 100.00 % | 100.00 % | 100.00 % | — |
| `hal/src/cpu.rs` | 100.00 % | 100.00 % | 100.00 % | — |
| `test-hal/src/console.rs` | 100.00 % | 100.00 % | 100.00 % | — |
| `test-hal/src/timer.rs` | 100.00 % | 100.00 % | 100.00 % | — |

## Scheduler gap triage

Uncovered in `kernel/src/sched/mod.rs` (actionable lines only; `//!` / `///` doc-comment lines are ignored below):

### High priority — T-011 candidates

**`start()` body (lines 426-473).** The function diverges (`-> !`), and `FakeCpu::context_switch` is a no-op, so test scaffolding has to be delicate but the dequeue / `task_states` / `current` writes before the switch are real state mutation worth exercising. **Recommendation:** split the prelude into a `start_prelude(sched) -> usize` helper that returns `next_idx`; `start` becomes `let next_idx = start_prelude(sched); IrqGuard; context_switch(...)`. Testable directly. Cost: small refactor, no behaviour change.

**`ipc_send_and_yield` entire body (lines 589-645).** Zero tests. The function has three observable outcomes — `Ok(Delivered)` with re-entrant yield, `Ok(Enqueued)` without yield, and propagated `Err(Ipc(…))` from a failed `ipc_send` — none is exercised. This was noted in T-007 as a deferred follow-up ("symmetric state-restore test"); coverage now confirms the gap. **Recommendation:** three host tests, analogous to T-007's two `ipc_recv_and_yield` tests. Route to T-011.

### Medium priority

**`unblock_receiver_on` panic branch (line 310).** The `panic!("scheduler invariant: ready queue full on unblock")` path is unreachable under the existing invariant but not exercised. Acceptable today; not worth a test since it's a panic branch on a provably-impossible condition.

**`SendOutcome::Enqueued` path in `ipc_send_and_yield` (lines 617-622).** When `outcome != Delivered`, `needs_yield = false` and the re-entrant `yield_now` is skipped. Covered by the ipc_send_and_yield three-case bundle above.

### Low priority / intentionally uncovered

- Lines 831-839, 873-875: FakeCpu/ResetQueuesCpu test-harness `instruction_barrier`, `wait_for_interrupt` stubs — no-op implementations no production code reaches. Fine.
- Lines 1131-1139, 1224: ResetQueuesCpu helpers only used in one test; partial coverage of its trait surface.
- Lines 184-186, 212-214, 282, 526, 536: mostly doc-comment / allow-pragma lines miscounted by the instrumentation.

## `kernel/src/cap/table.rs` sweep (40 uncovered regions)

Lines 90-92 (constructor edge), 202, 286, 320, 343, 351, 357, 367, 375, 381, 421, 448, 515, 526, 545, 557, 568, 573-576, 583, 590-596. Pattern: most look like error-return branches in `cap_derive`, `cap_drop`, `cap_take` — the failure modes are tested (clippy sees them as `Err` matches) but some specific error returns aren't triggered by current tests. Not a correctness concern; a completeness one. Route to T-011's missing-tests bundle if T-011 grows; otherwise accept at this level of coverage.

## Recommendations routed to T-011

When T-011 (Missing-tests bundle) is opened, its scope should include — in addition to the ADR-0019 / Phase-A-review items already listed:

1. Three `ipc_send_and_yield` tests: Delivered + unblock, Enqueued (no yield), Err propagation from `ipc_send`.
2. `start()` prelude refactor + direct test for the prelude's state mutation.
3. Targeted branch coverage for `cap/table.rs` error returns (≤ 5 tests cover most of the 40 gaps).

Non-recommendation: do NOT chase 100 % coverage. The current 94.4 % baseline is strong; the specific gaps named above are where real value lives.

## Next measurement

Next llvm-cov run: at T-011 closure (expected to push sched/mod.rs past 95 % regions and workspace past 96 %). Also re-run after T-008 architecture docs if it touches test scaffolding. Baseline-to-baseline diffs go into the same `reports/` folder with an ISO-date slug.
