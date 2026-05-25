# Sairgent

**This repository is archived.** Development ended in May 2026.

The full wind-down story is here: [I built a company in a box. Then I found one already built.](https://davidpengelley.substack.com/p/i-built-a-company-in-a-box-then-i)

Recommended successor for most use cases: [Hermes by Nous Research](https://github.com/nousresearch/hermes-agent).

For a tour of the codebase, read [`ARCHITECTURE.md`](./ARCHITECTURE.md).

What's still here, for anyone interested in the auditable, hierarchical, kernel-as-authority version of multi-agent orchestration:

- [`sairgent_kernel/`](./sairgent_kernel/) — Rust kernel: delegation policy, SQLite registry, audit chain, vault, orchestrator.
- [`apps/workspace/`](./apps/workspace/) — Tauri desktop shell.
- [`sairgent_harness/`](./sairgent_harness/) and [`sairgent_codex_harness/`](./sairgent_codex_harness/) — Python worker harnesses.
- [`.agents/skills/`](./.agents/skills/) — agent definitions (Manager / LeadIC / Specialist) as markdown.
- [`ops/runtime_event_bus_v1.md`](./ops/runtime_event_bus_v1.md) — the runtime contract between kernel and clients.

No issues, no pull requests, no support. MIT licensed. Fork freely.
