# Settings UX Architecture Spec

**Author**: Jimmy (CDO)
**Date**: 2026-04-04
**Status**: DRAFT -- awaiting review from Felicity (implementation) and Perry (prioritisation)

---

## 1. Settings Categories (5 groups)

| # | Label | Icon | What lives here |
|---|-------|------|-----------------|
| 1 | **Connections** | Plug / Link | API keys for AI providers (Anthropic, OpenAI, etc.). The single highest-priority section -- nothing works without it. |
| 2 | **Your Team** | Users / People | Agent visibility, nicknames, which agents are active vs benched. Future: custom agent creation. |
| 3 | **Appearance** | Paintbrush | Skin/theme selection (ASCII, Emoji, future skins). Animation speed. Grid density. Canvas zoom default. |
| 4 | **Notifications** | Bell | Sound on/off, desktop notification permission, which events trigger alerts (job complete, blocked, needs review). |
| 5 | **Advanced** | Wrench | Kernel connection diagnostics, data export, reset workspace, debug log toggle. Escape hatch for power users; hidden from first-run. |

Why 5, not 6: Fewer categories means the user can hold the full mental model in one glance. "Connections" is intentionally separated from "Advanced" because API keys are not an advanced concept -- they are the entry ticket.

---

## 2. Progressive Disclosure

### Surface-level (immediate toggle/select, no wizard)

- Skin selection (visual preview cards, one click to switch -- already built via `SkinSelector`)
- Notification toggles (on/off per event type)
- Animation speed slider
- Agent visibility toggles (show/hide on canvas)

### Guided wizard flows

**Connection Setup Wizard (3 steps)**

1. **Choose provider** -- Card grid showing supported providers (Anthropic, OpenAI, Google, etc.) with logos. User picks one. No dropdowns, no typing yet.
2. **Enter key** -- Single masked input field with a "Where do I find this?" collapsible hint linking to the provider's API key page. Paste-friendly (auto-trim whitespace). A "Test Connection" button that shows a spinner then a green checkmark or red error inline.
3. **Confirm** -- Summary card: "Anthropic connected. Your team is ready to work." with a prominent "Done" button. Offer "Add another provider" as a secondary action.

This wizard also serves as the **first-run onboarding flow** (see section 4).

**Agent Configuration (future, 2 steps)**

1. Pick agent from visual roster
2. Adjust role label, icon override, active/benched toggle

These are not raw forms. Each wizard step is a single focused question with a single primary action.

---

## 3. Layout Pattern

**Left sidebar navigation, right content pane.**

Rationale:
- The workspace app is a desktop Tauri window, not a mobile web page. Horizontal space is abundant.
- 5 categories fit comfortably in a vertical sidebar without scrolling.
- Sidebar nav means the user always sees where they are and can jump between sections with zero cognitive load. Tabs require scanning horizontally; accordions require scrolling to find the section header.
- The content pane scrolls independently, so long sections (e.g., multiple API connections) never push the nav off-screen.

### Sidebar spec

```
+------------------+---------------------------------------+
| [x] Close        |                                       |
|                  |          CONTENT PANE                  |
|  > Connections   |                                       |
|    Your Team     |  (scrollable, single section at a     |
|    Appearance    |   time, no mixing)                    |
|    Notifications |                                       |
|    Advanced      |                                       |
|                  |                                       |
+------------------+---------------------------------------+
```

- The Settings panel is a **full-viewport overlay** (not a route). It slides in from the right or fades over the workspace canvas. The canvas remains faintly visible behind a dark scrim -- this reinforces that Settings is a modal layer, not a separate app.
- Close button top-left of the sidebar (or Escape key).
- Active section highlighted in the sidebar with the same green accent used elsewhere in the terminal aesthetic.
- All settings auto-save on change (no global "Save" button). Show a brief "Saved" confirmation inline next to the changed field. For destructive actions (reset workspace), require explicit confirmation.

### Skin adaptation

The Settings overlay itself must respect the active skin's colour tokens. In ASCII skin: green-on-black mono type, box-drawing borders. In Emoji skin: the lighter palette. The overlay is not exempt from the brand.

---

## 4. First-Run / Onboarding

### Detection

On app launch, check whether any API connection is configured. If zero connections exist, the workspace canvas is replaced by the onboarding flow. The user cannot dismiss it -- there is nothing to show without a connection.

### Flow

1. **Welcome screen**
   - Headline: "Welcome to Sairgent"
   - Subhead: "Your AI team is ready. Let's connect them to a brain."
   - Single CTA button: "Set Up Connection"
   - No skip. No "later." The app does not function without this.

2. **Connection Wizard** (same 3-step wizard from section 2, launched inline)

3. **Success state**
   - The welcome screen dissolves. The workspace canvas fades in with agents at their desks.
   - A subtle pulse animation on the "Submit Job" button draws the eye to the next action.
   - Optional: a single tooltip callout pointing at "Submit Job" that says "Give your team their first task." Dismisses on click anywhere.

### Returning users

If connections exist but a key has expired or been revoked (detected via a failed health check), show a **non-blocking banner** at the top of the workspace: "Connection issue -- tap to fix." Tapping opens Settings > Connections with the broken connection highlighted in red.

---

## 5. Terminology Guide

| Internal / Technical | User-Facing Label |
|---------------------|-------------------|
| LLM provider, model config | **Connection** |
| API key | **API key** (this one is fine -- users know it from other apps) |
| Agent roster, agent registry | **Your Team** |
| Agent presence (READY/IDLE/COMPUTING) | **Status** (with labels: Working, Ready, Offline) |
| Skin / theme | **Look** (in casual references) or **Appearance** (in Settings) |
| SWO (Subordinate Work Order) | **Task** |
| HSM (Hierarchical State Machine) | Never exposed. If a status display is needed, use "Task progress." |
| Delegation | "Assigned to [agent name]" |
| Artifact | **Deliverable** |
| Inbox item | **Result** (or just show it in the tray without a category label) |
| Kernel | Never exposed. If diagnostics are needed, call it "Engine" in Advanced settings. |
| Orchestrator | Never exposed. |
| Bus / signals | Never exposed. |
| Vault | "Secure storage" (only in Advanced, if at all) |
| Agent org class (Manager/LeadIc/Specialist) | **Role** -- but only show the human-readable title (e.g., "Team Lead", "Researcher"), never the enum value |

### Tone rules for settings copy

- Use second person: "Your team", "Your connections"
- Use active voice: "Test connection" not "Connection will be tested"
- Error messages state the problem and the fix: "This key was rejected by Anthropic. Double-check it and try again."
- Never show stack traces, error codes, or JSON in the UI. Log those to the debug console in Advanced.

---

## 6. Interaction Details

### Settings entry point

A gear icon button in the Header bar, placed between the Activity button and the Skin Selector (which will move into Settings > Appearance once this ships). The current `SkinSelector` dropdown in the header should be replaced by the gear icon to reduce header clutter.

### Keyboard

- `Cmd/Ctrl + ,` opens Settings (standard desktop convention)
- `Escape` closes Settings
- `Tab` moves between form fields within a section
- Wizard steps advance on `Enter` when the primary field is filled

### Transitions

- Settings overlay: 200ms ease-out slide from right
- Wizard step transitions: 150ms crossfade
- Auto-save confirmation: fade in, hold 1.5s, fade out

---

## 7. Open Questions for Perry / Felicity

1. **Persistence layer**: Settings currently use `localStorage` (see `useSkin.ts`). Should we move to Tauri's `app_data_dir` for a unified config file, or keep localStorage for non-sensitive prefs and vault for keys?
2. **Connection health checks**: Should we ping providers on Settings open, or run a background check on an interval? Background checks add complexity but catch expired keys proactively.
3. **Skin selector migration**: Confirm that removing the header dropdown in favour of Settings > Appearance is acceptable. The tradeoff is one extra click to switch skins, but a cleaner header.
