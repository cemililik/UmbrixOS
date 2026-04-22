# Miri aliasing validation — 2026-04-23

`cargo +nightly miri test` against the full workspace test suite. This is the first run — the first *dynamic* test of ADR-0021's claim that "no `&mut` reference to `Scheduler<C>` / `EndpointArena` / `IpcQueues` / `CapabilityTable` is alive across `cpu.context_switch`".

## Result

**111 / 111 host tests pass under Miri** with Stacked Borrows checking enabled.

| Crate | Tests | Result |
|---|---|---|
| `tyrne-kernel` | 77 | ✅ |
| `tyrne-test-hal` | 34 | ✅ |
| `tyrne-hal` | 0 (no tests) | ✅ |

No undefined behaviour reached at runtime under Miri's interpretation. The bridge's momentary `&mut` materialization discipline, the split-borrow on `(*sched).contexts[current_idx]` vs. `contexts[next_idx]`, and the per-phase re-acquisition pattern in `ipc_recv_and_yield` — all survive the Stacked Borrows checker.

## Pre-fix find (now closed)

The first run caught **one real Stacked Borrows violation**, and it was my own doing in the T-007 test-helper code:

```text
error: Undefined Behavior: trying to retag from <1907727> for Unique permission
       at alloc621333[0x0], but that tag does not exist in the borrow stack
       for this location
    --> kernel/src/sched/mod.rs:1156:25
```

The `ResetQueuesCpu` test helper cached a raw pointer to the test's stack-local `IpcQueues` by doing `core::ptr::from_mut(&mut queues)`. The test body later created a *second* `core::ptr::from_mut(&mut queues)` to pass into `ipc_recv_and_yield`. Per Stacked Borrows, the second `&mut queues` borrow — even though it's immediately consumed by `from_mut` — pops the stack and invalidates the tag the first raw pointer carries. When `context_switch` then dereferenced the cached pointer, the tag was already dead.

**Fix:** derive each raw pointer exactly once, at the top of the test, and reuse the `*mut` value in both the helper's field and the bridge call. Commit lands with this change.

Notable: this is the **same aliasing-discipline error** ADR-0021 eliminates from the production bridge — applied inside a test helper, where I hadn't extended the discipline yet. The T-006 retro's lesson ("trace the call graph") applies here too: when a test helper caches a pointer derived from a stack local, every subsequent re-derivation invalidates it.

## What this validates

Miri's Stacked Borrows model is stricter than Tree Borrows (and both are stricter than Rust's currently-shipped compiler). A clean Miri pass is strong — but not conclusive — evidence that:

- **The production bridge's `&mut`-discipline holds dynamically, not just by inspection.** Every momentary `&mut *sched`, `&mut *ep_arena`, etc. inside the bridge drops before the switch site as claimed; every post-switch re-borrow picks up a fresh tag.
- **The split-borrow on `(*sched).contexts[i]` vs `contexts[j]` is sound.** The two indices are distinct by construction and Miri confirms the compiler sees them that way.
- **UNSAFE-2026-0012's retirement is dynamic-test-validated**, not only paper-reviewed. Prior to today, ADR-0021's claim was defended by static analysis (read the code, argue the borrow scopes). Now it is defended by a test tool that actually traces every retag.

## What this does NOT validate

Miri cannot observe what the optimizer does. The BSP (`tyrne-bsp-qemu-virt`) is a bare-metal binary, not tested under Miri here; its `&mut` discipline is inherited from the kernel bridge (Miri validates the called code) but the BSP-local pointer plumbing (`StaticCell::as_mut_ptr`, the task-body pointer threading) is only validated statically. Building the BSP under Miri would require a custom target + `no_std` shim that the current Miri on aarch64-unknown-none does not support out of the box.

Also: Miri is not preemption-aware. Every test here runs on a single thread with `FakeCpu` / `ResetQueuesCpu` context switches as no-ops. When preemption lands, the discipline of "no `&mut` live across switch" has to hold under actual concurrent execution — the Phase C SMP ADR will re-open this question, possibly requiring a thread-safe variant of the test harness.

## Integration recommendation

Add `cargo +nightly miri test` to the CI matrix (T-R6 / future K3-7). Roughly ~15 seconds to ~10 minutes of wall time per run, depending on how much IPC state is exercised. Run on every PR that touches `kernel/src/sched/` or `kernel/src/ipc/`. A Miri regression is a hard stop — it means someone reintroduced an aliasing hazard.

CI target command:

```bash
cargo +nightly miri test --workspace --exclude tyrne-bsp-qemu-virt
```

(Exclude the BSP for the same reason llvm-cov does — `no_std` + `no_main` is unbuildable on the host target.)

## Next measurement

Next Miri run: when any new `unsafe` block or new raw-pointer API lands in the kernel or test-hal. Tree Borrows (`-Zmiri-tree-borrows`) is a future refinement once T-R6 adds CI — stricter still than Stacked Borrows, but known to flag some patterns Stacked Borrows accepts. If/when we move to it, expect fresh surface findings similar to the test-helper one closed by this report.
