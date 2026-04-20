# Current focus

A short pointer file updated as work progresses. For the full plan see [`phases/`](phases/); for the process see [ADR-0013](../decisions/0013-roadmap-and-planning.md).

---

- **Active phase:** A — Kernel core on QEMU `virt`.
- **Active milestone:** A2 — Capability table foundation.
- **Active task:** [T-001 — Capability table foundation](../analysis/tasks/phase-a/T-001-capability-table-foundation.md) (status: **In Progress**).
- **Working branch:** `development`.
- **Last completed milestone:** A1 — Bootable skeleton, on 2026-04-20 (commit `2944e7d`).
- **Last review:** none yet; the first business review will accompany A2 completion.
- **Next review trigger:** completion of milestone A2 → business-review artifact in [`../analysis/reviews/business-reviews/`](../analysis/reviews/business-reviews/).

## Notes

- Work on T-001 happens on the `development` branch; PR opens against `main`.
- [ADR-0014](../decisions/0014-capability-representation.md) is Accepted; implementation can proceed.
- The kernel currently boots on QEMU virt; host tests pass (34/34). T-001 adds new host tests only; no runtime regression expected.
- The maintainer updates this file when the active task changes. AI agents update it when they move a task to `In Progress` or `Done` via the [`start-task`](../../.claude/skills/start-task/SKILL.md) and [`conduct-review`](../../.claude/skills/conduct-review/SKILL.md) skills.
