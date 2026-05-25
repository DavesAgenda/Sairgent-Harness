# Runtime Event Bus v1

## Purpose

The runtime event bus is the canonical interaction protocol between the Sairgent runtime and its consumers.

Consumers may include:

- desktop UI
- future hosted web UI
- future alternate skins
- future external interaction adapters

The bus is intentionally runtime-semantic. It does not encode panel state or view-specific layout concerns.

## Build Rule

Felicity and any engineering implementation touching operator/runtime state must build to this event-driven pub/sub model by default.

Required posture:

- treat `runtime_bootstrap` + `runtime-signal` + `runtime_replay` as the canonical runtime boundary
- extend the bus contract before adding route-local refresh logic, polling shortcuts, or client-specific side channels
- publish projection-safe state changes once and let consumers reduce them, rather than teaching each client how to re-query the runtime ad hoc
- reserve direct bootstrap reloads for explicit resync conditions such as `runtime.sync.required`, not normal success paths

## Core Model

### Bootstrap

`runtime_bootstrap` returns the current runtime snapshot:

- `cursor`
- `hsmStatus`
- `runtimeContext`
- `queue`
- `roster`
- `approvals`
- `recentArtifacts`
- `recentFeed`
- `attentionSummary` (when the inbox projection ships)

Manager/org configuration extension should treat dedicated detail/config queries as the authoritative editor hydration path, then reduce live changes from bus signals rather than polling route-local state.

### Signals

`runtime-signal` emits envelopes with:

- `id`
- `correlationId`
- `source`
- `principal`
- `audience`
- `redactionClass`
- `occurredAt`
- `cursor`

Signal kinds in v1:

- `runtime.status.changed`
- `feed.message.appended`
- `project.upserted`
- `swo.upserted`
- `approval.upserted`
- `approval.removed`
- `agent.presence.changed`
- `artifact.created`
- `attachment.upserted`
- `delivery.status.changed`
- `project.status.updated`
- `project.activity.appended`
- `project.output.created`
- `recurring.template.upserted`
- `recurring.run.upserted`
- `decision_log.appended`
- `inbox.item.upserted`
- `inbox.item.resolved`
- `runtime.sync.required`

Required extension for manager/org policy work:

- `agent.configuration.updated`
- `agent.hired`
- `agent.reporting_line.updated`
- `team.goal.upserted`
- `team.goal.archived`
- `delegation.decision.recorded`
- `team.gap.detected`

Publication rule for v1:

- start-of-work and terminal-state mutations must publish the full operator-safe response set needed to keep projections live
- attachments and generated artifacts must use different signal kinds
- approval removal must be explicit; consumers should not rely only on inference from terminal SWO state
- projection failures must fail closed as `runtime.sync.required`, never as partially-redacted ad hoc payloads

Producer rule for v1:

- mutation paths must publish through the shared Tauri publisher helpers, not through route-local one-off signal sequences
- the same helper family is responsible for:
  - work-start publication
  - terminal SWO publication
  - approval upsert/removal diffs
  - project output/activity publication
  - agent presence transitions
  - projection-failure fallback
- configuration and org-policy mutations must publish the matching configuration/delegation signals from the same shared publisher layer

### Replay

`runtime_replay` accepts a prior cursor and returns all retained signals after that cursor.

Retention is best-effort in v1 and intended for reconnect smoothing, not long-term event sourcing. Consumers must treat `runtime_bootstrap` as the authoritative resync path.

Replay safety rule in v1:

- desktop replay must only return `operator_safe` signals
- external-adapter scoped events are excluded from desktop replay

### Commands

Runtime commands carry:

- `commandId`
- `correlationId`
- `source`
- `principal`

Canonical command families in v1:

- `chat.send`
- `project.create`
- `work_order.submit`
- `swo.retry`
- `approval.decide`
- `swo.manual_close`
- `heartbeat.trigger`

The first dedicated runtime-bus command handler delivered in this sprint is `queue_review_decide`.

Later dependent inbox work adds:

- `inbox_list`
- `inbox_acknowledge`
- `inbox_resolve`

Required extension for manager/org policy work:

- `agent.configuration.update`
- `team.goal.upsert`
- `team.goal.archive`
- `agent.hire.submit`
- `agent.reporting_line.set`

## Security Rules

- Signals are projection-safe payloads, not raw execution traces.
- Raw worker stdout/stderr must remain internal and out of shared-bus payloads.
- Commands must include source/principal metadata.
- Commands must support idempotency through command/correlation identifiers.
- Authorization must be enforced at command handlers, not in individual clients.
- `audience` and `redactionClass` exist so future adapters can safely filter and project events.
- Desktop-facing `runtime-signal` must carry only `operator_safe` payloads in v1.
- `secret_adjacent` and `internal_only` data stay out of `runtime-signal`; they may be recorded in audit storage only.
- If the publisher cannot safely build a projection-safe event, it must emit `runtime.sync.required` rather than leak partial/raw data.

## Audit Rules

- Important command receipts must be recorded in audit storage.
- Important emitted signals must be recorded in audit storage.
- `correlationId` is the join key across command receipt, downstream runtime activity, and emitted signal projections.

## Compatibility

During migration, the following legacy events remain available:

- `chat-reply`
- `hsm-status`
- `hsm-error`

These are compatibility shims only. New consumers should prefer `runtime_bootstrap`, `runtime_replay`, and `runtime-signal`.

## Consumer Guidance

Desktop/web/skinned clients should:

1. Call `runtime_bootstrap`
2. Start listening to `runtime-signal`
3. Optionally call `runtime_replay` after reconnect using the last cursor
4. Resync with `runtime_bootstrap` if a `runtime.sync.required` signal is received

Additional consumer rules:

- queue reducers must remove stale approvals when `approval.removed` is received
- project projections must react to SWO, approval, activity, output, and artifact events, not just artifacts
- inbox consumers should hydrate from inbox/bootstrap data and stay current from `inbox.item.upserted` and `inbox.item.resolved`
- manager/org configuration views should hydrate from dedicated config/detail queries and stay current from `agent.configuration.updated`, `team.goal.*`, `agent.hired`, and `agent.reporting_line.updated`
- manager oversight views should consume `delegation.decision.recorded` and `team.gap.detected` rather than inferring policy outcomes from free-form review text

External adapters should:

- consume only projected bus events
- never bypass shared command handlers
- translate semantic actions into channel-native UI affordances without changing command semantics
