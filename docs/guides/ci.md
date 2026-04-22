# CI: what runs, where, and when

This guide describes the GitHub Actions pipeline configured in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml). It is the first external gate beyond self-review and agent-review. When the CI is red, the commit is not mergeable, even if local checks passed.

## Jobs

| Job | Toolchain | Wall time (expected) | Fails on |
|-----|-----------|----------------------|----------|
| `lint-and-host-test` | stable + `rustfmt` + `clippy` | ~2 min | `cargo fmt --check` diff, any clippy warning, any failing host test |
| `kernel-build` | stable + `aarch64-unknown-none` + `clippy` | ~1 min | `cargo kernel-build` error, any kernel-clippy warning |
| `miri` | nightly + `miri` component | ~10–15 min | Any Stacked Borrows violation in `cargo +nightly miri test --workspace --exclude tyrne-bsp-qemu-virt` |
| `coverage` | nightly + `llvm-tools-preview` + `cargo-llvm-cov` | ~3–5 min | **Never** (informational only, `continue-on-error: true`) |

All jobs run on `ubuntu-latest`. Each caches cargo registry + build artefacts keyed by `Cargo.lock` hash, so warm runs are far faster than first runs.

## Triggers

- **Every push** to `main` or `development`.
- **Every PR** targeting `main` or `development`.

Concurrent runs on the same branch cancel each other — only the latest commit's verdict matters.

## Philosophy

The CI matrix mirrors what a contributor should run locally before opening a PR:

| Local command | CI job |
|---|---|
| `cargo fmt --all -- --check` | `lint-and-host-test` (step 1) |
| `cargo host-clippy` | `lint-and-host-test` (step 2) |
| `cargo host-test` | `lint-and-host-test` (step 3) |
| `cargo kernel-build` | `kernel-build` |
| `cargo kernel-clippy` | `kernel-build` |
| `cargo +nightly miri test --workspace --exclude tyrne-bsp-qemu-virt` | `miri` |
| `cargo llvm-cov --workspace --exclude tyrne-bsp-qemu-virt --summary-only` | `coverage` |

If you pass all seven locally, CI should pass too. If CI fails on something you passed locally, the most common cause is that your local `rustup default` is pinned to a version CI doesn't have — run `rustup update stable` and retry.

## Why is `tyrne-bsp-qemu-virt` excluded from Miri and coverage?

The BSP is a bare-metal `no_std` + `no_main` binary whose panic handler conflicts with `std`'s `panic_impl` lang item when built for the host target (which Miri and llvm-cov both require). BSP code is exercised indirectly via the QEMU smoke test; automating that runs under CI is a T-009 follow-up (the timer init task) — once the kernel can produce a finish-signal, QEMU can exit non-zero on mismatch and CI can assert the trace.

## When does the coverage job start gating?

Today `coverage` is informational: `continue-on-error: true` in the workflow. After T-011 closes (which raises `sched/mod.rs` past 90 % and the workspace past 96 %), the plan is to flip this job to enforce a floor. The floor should be slightly below the measured baseline so regressions trip the gate but normal churn does not.

## Adding a new check

1. Add a job to [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).
2. Keep the fast-lane job order stable — `lint-and-host-test` must remain first so red PRs fail quickly.
3. If the check requires a nightly feature, put it in its own job (not folded into `lint-and-host-test`).
4. Update this guide's table.

## Nightly pinning

Miri and cargo-llvm-cov currently use `nightly` (floating). If a nightly regression breaks the pipeline, temporarily pin in the workflow:

```yaml
rustup toolchain install nightly-YYYY-MM-DD --component miri
```

Open an issue with the pinned date so the pin is not forgotten.
