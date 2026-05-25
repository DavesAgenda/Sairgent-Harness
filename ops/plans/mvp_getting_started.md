# MVP Getting Started: Three Use Cases, Tool Access, and Onboarding

**Reviewed by**: Oracle (CPO), Jimmy (CDO), Kryptonite (CISO)
**Version**: 2.0 — post-review synthesis

## Context

Sairgent has sophisticated infrastructure (Rust kernel, audit trail, delegation, vault, SWO lifecycle) but a new user currently hits a wall after onboarding: they have an agent team but no clear path to value. Tool access is opaque — agents can think but can't reliably *do* things. The seed JSON is hardcoded to `codex`/`gpt-5-codex` (which may not match the user's provider), the vault key is a dummy string, and no tool API keys are collected during onboarding.

**Target**: A GitHub evaluator clones the repo, builds, and reaches "wow" in under 5 minutes. The wow is watching Perry delegate a task across multiple agents and synthesize the result.

---

## The Three Getting Started Use Cases

### Use Case 1: "Meet Your Team" (Zero-Config Chat)
**What**: User clicks a suggested prompt on HomeRoute ("What can each agent on my team do?"). Perry responds with a team introduction.
**Why it's first**: Requires only an LLM key. Proves the system boots and agents can reason.
**Differentiator shown**: Agent personas, org structure, the chat panel.
**Implementation**: Suggested prompts on HomeRoute empty state (NOT a starter SWO — auto-play removes user agency).

### Use Case 2: "Delegate a Task" (Multi-Agent Fan-Out)
**What**: User submits a work order ("Draft a competitive positioning brief for [my product]"). Perry triages, delegates to Cat + Lois. They execute. Perry synthesizes. Result surfaces in inbox for review.
**Why it's second**: Showcases the core differentiator — multi-agent delegation + synthesis.
**Differentiator shown**: SWO lifecycle, manager synthesis gate, operator approval gate, audit trail.
**Implementation**: Already works via `submit_work_order`. Needs: suggested prompts for discoverability + toast notification on `swo.upserted` so user sees delegation happening without navigating away.

### Use Case 3: "Research with Web Search" (Tool-Augmented Agents)
**What**: User asks Lois to research something requiring live web data. Lois uses Tavily, produces a cited dossier.
**Why it's third**: Requires one additional API key (Tavily free tier).
**Differentiator shown**: Capability-gated tool access (only Lois has WebSearch), evidence-first research.
**Implementation**: Tool key collected in onboarding step 3, auto-bound to eligible agents.

---

## Onboarding: 4 Steps (~2 Minutes to First Agent Interaction)

### Current Flow (3 steps)
1. Provider selection + API key -> `settings_save`
2. Model selection -> `settings_save`
3. Seed + Launch -> `resetRuntime` + `bootFromKeychain`

### Revised Flow (4 steps)

#### Step 1: Provider + API Key (MODIFY)
**Keep**: Provider selection, API key input.
**Add**: Validation spinner — call new `validate_llm_credential` Tauri command (minimal API call, 10s timeout). On failure, show typed error message (not raw HTTP): `invalid_key | quota_exceeded | network_error | timeout`.
**Security (Kryptonite M-1, M-2)**: Never log the key value. Only send key to hardcoded endpoint for the selected provider slug. No arbitrary base URLs.
**Files**: `lib.rs` (new command), `adapter.ts` (new method), `OnboardingWizard.tsx` (validation call + error taxonomy)

#### Step 2: Context (NEW — lightweight, 2 fields only)
```
"What are you building?" (title)
"Name your team's workspace so agents know your context" (subtitle)

Company / Project Name:
  [___________________________]

What does it do? (one or two sentences)
  [___________________________]

[Skip for now]  [Save & Continue ->]
```
**Only 2 fields**: name + summary. Operating principles and non-goals are deferred to Settings > Context (Oracle: "power-user features that add cognitive load for minimal first-session value"). The kernel works fine with empty arrays.
**Security (Kryptonite H-1)**: Rust-level enforcement: `company_name` max 200 chars, `company_summary` max 2000 chars. Strip null bytes, ANSI escapes, Unicode BIDI overrides (U+202A-U+202E, U+2066-U+2069). When injecting into prompts, use structured delimiters: `<company_context>...</company_context>`.
**Files**: `OnboardingWizard.tsx` (new step), `lib.rs` (seed spec override + input validation)

#### Step 3: Models + Tools (MERGED — one screen)
```
"Configure models and tools" (title)

Default Model:
  [dropdown: discovered models from default provider]

Optional: Web Search (recommended)
  Tavily — free tier, 1000 searches/month
  [Get free key -> tavily.com]
  [API key input field        ]

[Continue ->]
```
**Merged** per Oracle and Jimmy: both are "configure external services." Model selection is required, tool key is optional. Drop the "Sairgent Agent Model" selector to Settings (Jimmy).
**Auto-binding**: After seeding (Step 4), call `auto_bind_tools_for_provider(provider_slug)` for any tool keys saved here. This queries agents with matching capability (`WebSearch`) and binds automatically.
**Security (Kryptonite M-5)**: Auto-bind uses strict allowlist mapping (tavily -> WebSearch, exa -> WebSearch, nothing else). Emit audit event per binding. Post-boot status shows which agents received bindings.
**Files**: `OnboardingWizard.tsx` (merged step), `lib.rs` (auto-bind command)

#### Step 4: Seed + Launch (MODIFY — includes brief status confirmation)
Click "Create Team & Launch Sairgent" -> seed with overrides -> brief inline confirmation -> auto-transition to HomeRoute.

**Seed overrides applied before seeding**:
- `company_name` + `company_summary` from Step 2
- `default_provider` + `default_model` from Steps 1+3
- Each agent's `provider`/`model` overridden to user's chosen defaults
- Auto-bind tool keys from Step 3

**Inline confirmation** (NOT a separate wizard step — Jimmy: "the reward after the progress bar completes"):
```
8 agents created. LLM connected. [Web Search: configured / not configured]
```
Then auto-transition to HomeRoute after 1s.

**Security (Kryptonite L-5)**: No starter SWO with interpolated user input. Suggested prompts on HomeRoute instead.
**Files**: `syllogism_runtime_seed.json` (rename to `default_seed.json`), `lib.rs` (seed spec override + auto-bind)

---

## HomeRoute: The Actual "Wow" (Post-Onboarding)

When the user lands on HomeRoute for the first time (no SWOs yet), show:

```
Getting Started
--------------------------------------------------
Try asking your team:

  [What can each agent do?]
  [Research the top 3 competitors to X]
  [Create a go-to-market brief for my product]

--------------------------------------------------
Tell your team what you're building (dismissible single-field banner):
  [e.g., "We're a consulting firm helping enterprises adopt AI"]
  [Save]
```

- Suggested prompts are clickable chips that populate the chat input.
- The single-field banner feeds `company_summary` for users who skipped Step 2.
- Dismissible — once dismissed or filled, it doesn't come back.
- **Team goals** are deferred to Agents > Charter panel (no new user knows what delegation fit scoring is).
- **Toast notification** on `swo.upserted` signal so the user sees delegation happening without navigating.

**Files**: `apps/desktop/src/ui/HomeRoute.tsx`

---

## Context Establishment: Full Form in Settings (Post-Onboarding)

After first use, the user can edit the full context in Settings > Context:

- Company / Project Name
- Summary (what it does)
- Operating Principles (add/remove, max 20, max 500 chars each)
- Non-Goals (add/remove, max 20, max 500 chars each)
- Total combined principles + non-goals: max 5000 chars (Kryptonite H-2: prevents prompt size inflation)

Changes emit `runtime.sync.required` to trigger consumer re-bootstrap (event bus compliant).

**Security**: Same Rust-level input validation as onboarding (length limits, control char sanitization, structured delimiters).

**Files**: `apps/desktop/src/ui/SettingsRoute.tsx` (new "Context" section)

---

## How Context Flows Into Agent Work

After context is set, every agent prompt includes:
1. **Company context** (via `SairgentRuntimeSnapshot`): `<company_context>...</company_context>` — injected in `build_sairgent_chat_prompt` and `build_triage_context`
2. **Agent identity** (via env vars): `AGENT_PERSONA_PROMPT` + `AGENT_RAISON` — injected in every harness prompt builder
3. **Team goals** (via `list_descendant_team_goals`): Used in delegation fit scoring and heartbeat prompts
4. **Operating principles + non-goals** (NEW, when set): Appended to company context in runtime snapshot

This means: context choices directly steer which agent gets delegated what, how agents reason about priorities, and what they consider in/out of scope.

---

## MVP Tool Access Design

### Tier 0: Zero-Config (Just LLM Key)
- Chat / Reason / Plan, Delegate, File I/O, Hire, Skills — all already wired.

### Tier 1: One Additional API Key
- **Tavily Web Search** (`web_search_tavily`): Free tier at tavily.com (1000 searches/month)
- Auto-bound to eligible agents during onboarding. No manual Agent > Tools > Bind needed.

### Tier 2: MCP (Post-MVP)
- Framework fully built. Do not pre-bundle any servers. Document as extensibility path in README.

---

## Critical Security Fixes (Kryptonite Mandatory Items)

### FIX-1: Vault Key (CRITICAL)
**Current**: `lib.rs:5737` — `"dummy_vault_key_that_is_32_bytes"` hardcoded.
**Fix**:
1. On first boot, generate random 32-byte key via `rand::thread_rng().gen::<[u8; 32]>()`
2. Store in keyring at account `vault_key` (service `com.sairgent.deck.v2`)
3. On subsequent boots, load from keyring
4. **Fallback**: `~/.sairgent/vault.key` with 0600 perms, logged as "degraded security mode"
5. **Migration (Kryptonite L-1, MANDATORY)**: On first boot after fix, detect existing `storage/kernel_registry.sqlite`. Attempt decrypt with new key. If fails, attempt with legacy dummy key. If legacy works, re-encrypt all vault entries with new key.
6. Add `vault.key` to `.gitignore`

**Files**: `lib.rs` (init_kernel), `.gitignore`

### FIX-2: secure_bundle.json Dual-Write (CRITICAL — Kryptonite CRITICAL-1)
**Current**: `lib.rs:3198-3221` — `persist_bundle_to_file` writes ALL API keys (OpenAI, Anthropic, Tavily, sidechannel token) as plaintext JSON to `~/.sairgent/secure_bundle.json`. This happens ALWAYS, even when keyring succeeds. On Windows, no permission restriction (the `#[cfg(unix)]` guard means Windows gets default permissions).
**Fix**: Only write the file when keyring is unavailable. Check keyring write success before calling `persist_bundle_to_file`. If keyring succeeds, skip file write entirely.
**Files**: `lib.rs` (persist_secure_settings_bundle)

### FIX-3: Seed Spec Provider Override
**Current**: Seed JSON hardcodes `"default_provider": "codex"`, `"model": "gpt-5-codex"`.
**Fix**: Override from user's `config.toml` selections before seeding.
**Files**: `lib.rs` (runtime_reset_and_seed_default)

### FIX-4: Seed File Rename (Oracle)
**Current**: `syllogism_runtime_seed.json` with profile_id `"syllogism-runtime-v2-codex-manual"`.
**Fix**: Rename to `default_seed.json` with generic profile_id. Context override handles personalization.
**Files**: `00_Context/Seeds/`, `lib.rs` (path reference)

---

## Model Management (Phase 3 — Post-Core-MVP)

### Settings > Models Section
- **Default model dropdown** from discovered models (existing `provider_discover_models`)
- **Agent model table**: read-only with checkboxes, shows each agent's current model
- **Bulk reassign**: select agents + model dropdown + Apply. Calls new `agents_bulk_update_model` Tauri command.
- **No model allowlist for MVP** (Oracle: "over-scoped")
- **Security (Kryptonite M-9)**: Validate all agent_ids exist in registry. Fail entire batch if any invalid. Audit event per agent updated.

**Files**: `SettingsRoute.tsx`, `lib.rs` (new bulk command), `adapter.ts`

---

## Event Bus Compliance

All new mutations publish through `publish_operator_safe_signal` per `ops/runtime_event_bus_v1.md`.

| New Command | Signal(s) Emitted | Notes |
|---|---|---|
| `validate_llm_credential` | None (read-only) | |
| `auto_bind_tools_for_provider` | `agent.configuration.updated` per agent | + audit event per binding |
| `agents_bulk_update_model` | `agent.configuration.updated` per agent | + audit event per update |
| Seed with context override | Full bootstrap reload | Existing behavior |
| Context edit in Settings | `runtime.sync.required` | Triggers consumer re-bootstrap |
| Toast on delegation | Consumes existing `swo.upserted` | No new signal needed |

No new signal kinds needed. All mutations use existing `publish_operator_safe_signal` helper.

---

## File Change Summary

| File | Changes |
|------|---------|
| `apps/desktop/src-tauri/src/lib.rs` | FIX-1 vault key (keyring + migration), FIX-2 stop dual-write, FIX-3 seed provider override, `validate_llm_credential` command, `auto_bind_tools_for_provider` command, input validation for context fields |
| `apps/desktop/src/ui/OnboardingWizard.tsx` | 4-step wizard: add validation spinner, context step (2 fields), merge models+tools, inline status confirmation |
| `apps/desktop/src/ui/HomeRoute.tsx` | Getting started suggested prompts, dismissible context banner, toast on delegation |
| `apps/desktop/src/ui/SettingsRoute.tsx` | New "Context" section (full form: name, summary, principles, non-goals) |
| `apps/desktop/src/desktop/adapter.ts` | Add `validateLlmCredential()`, `autoBindToolsForProvider()` methods |
| `sairgent_kernel/src/orchestrator.rs` | Structured delimiters for context injection, include principles+non_goals in snapshot |
| `sairgent_kernel/src/kernel.rs` | Store principles + non_goals in runtime_metadata |
| `00_Context/Seeds/default_seed.json` | Renamed from syllogism, generic profile_id |
| `.gitignore` | Add `vault.key` |

---

## Verification

1. **Onboarding E2E**: Fresh install -> provider + key (validation spinner) -> context (name + summary) -> models + tools -> seed + launch -> auto-transition to HomeRoute -> see suggested prompts
2. **Context injection**: After onboarding with custom name/summary -> chat with Perry -> response references user's context, not "Syllogism"
3. **Tool auto-bind**: After onboarding with Tavily key -> Agents > Lois > Tools tab -> Tavily already bound
4. **UC1**: Click suggested prompt on HomeRoute -> Perry responds with team overview
5. **UC2**: Submit work order -> see toast notification -> watch SWO lifecycle -> review in inbox
6. **UC3**: Ask Lois to research with web search -> cited results
7. **Vault**: Restart app -> kernel boots with keyring-stored vault key, not dummy
8. **Vault migration**: Existing install with dummy key -> upgrade -> data preserved, re-encrypted
9. **secure_bundle.json**: After onboarding with keyring available -> `~/.sairgent/secure_bundle.json` does NOT exist (or is empty)
10. **Provider override**: Seed agents use user's chosen provider/model
11. **Reset path**: Settings > Reset Sairgent -> clears state, returns to onboarding

---

## Sequencing

**Phase 1 — Security + Critical Blockers** (must ship before GitHub):
1. FIX-1: Vault key (keyring + migration path)
2. FIX-2: Stop secure_bundle.json dual-write
3. FIX-3: Seed spec provider/model override
4. FIX-4: Rename seed file to generic
5. `validate_llm_credential` command (with error taxonomy, no key logging, hardcoded endpoints)
6. `auto_bind_tools_for_provider` command (strict allowlist, audit trail)
7. Input validation for context fields (Rust-level length limits + control char sanitization)

**Phase 2 — Onboarding + Discoverability** (the MVP experience):
8. 4-step onboarding wizard (provider, context, models+tools, seed+launch)
9. HomeRoute: suggested prompts + dismissible context banner
10. Toast notification on `swo.upserted` for delegation visibility
11. Settings > Context section (full form for principles + non-goals)
12. README with installation, onboarding walkthrough, and 3 use cases

**Phase 3 — Model Management + Polish**:
13. `agents_bulk_update_model` Tauri command
14. Settings > Models section (discovery, agent table, mass reassign)
15. Settings > Reset Sairgent
16. Capability badges on agent cards
