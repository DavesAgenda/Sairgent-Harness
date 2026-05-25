# CSS to Tailwind/Radix/shadcn Migration Plan
## Next-Gen Modern Project Management UI

---

## Executive Summary

**Current State:** Sairgent desktop uses a **custom CSS design system** — a well-architected two-layer token system (primitive palette → semantic tokens) with ~2,200 lines of hand-rolled CSS. Tailwind v4, Radix UI, shadcn/ui, CVA, and Lucide React are installed but at 0% usage.

**Vision:** Deliver a **Linear/Vercel-caliber dark PM interface** — fast, keyboard-first, information-dense, with fluid micro-animations and a cohesive component system. Not just a CSS swap; a product quality step-change.

**Decision:** Full migration in **6 focused sprints** (not calendar weeks — sprint = shippable increment). Desktop-first. SaaS in a separate phase.

**Owner:** Jimmy (CDO) extended with component architecture expertise. Felicity (CTO) reviews React/TS patterns per sprint.

---

## 1. Gap Analysis: Current Plan vs. Next-Gen Requirements

### 1.1 What the Original Plan Got Right
- Two-phase token preservation (semantic CSS vars → Tailwind theme)
- Component replacement map (Modal → Dialog, StatusPill → Badge)
- Accessibility via Radix primitives
- Desktop-first sequencing

### 1.2 Critical Gaps in the Original Plan

| Gap | Impact | Resolution |
|-----|--------|-----------|
| **Tailwind v4 is CSS-first** — no `tailwind.config.ts` | Phase 1 would fail | Use `@theme` directive in CSS |
| **No command palette** | Missing core PM UX pattern | Add `cmdk` in Sprint 1 |
| **No data table system** | Work Orders, Agents routes need sortable/filterable tables | Add TanStack Table v8 |
| **No drag-and-drop** | Kanban board view unusable | Add `@dnd-kit/core` |
| **No virtual scrolling** | Large agent/SWO lists will degrade | Add TanStack Virtual |
| **No toast/notification system** | No feedback on async actions | Add `sonner` |
| **No motion library** | Static UI feels dated vs. Linear/Vercel | Add `motion` (Motion One) |
| **No context menus** | Right-click actions missing everywhere | Add Radix ContextMenu |
| **No hotkey system** | Keyboard-first UX requires declarative hotkeys | Add `@github/hotkey` or `hotkeys-js` |
| **Phase 3 (CSS cleanup) after Phase 2 (components)** | Creates parallel debt | Run CSS migration in-stride with each component |
| **No design language spec** | Subjective quality bar, inconsistent results | Define visual language below |

### 1.3 Tailwind v4 Migration — Key Differences from v3

Tailwind v4 uses **CSS-first configuration**. There is no `tailwind.config.ts`.

```css
/* apps/desktop/src/index.css */
@import "tailwindcss";

@theme {
  /* Map our existing semantic tokens directly */
  --color-surface-base: var(--surface-base);
  --color-surface-raised: var(--surface-raised);
  --color-accent: var(--color-accent);
  --color-text-primary: var(--text-primary);
  --color-text-secondary: var(--text-secondary);
  --color-status-active: var(--color-active);
  --color-status-blocked: var(--color-blocked);
  --color-status-review: var(--color-review);
  --color-status-complete: var(--color-complete);
  --color-status-critical: var(--color-critical);
  --color-status-milestone: var(--color-milestone);
  /* ... all semantic tokens as Tailwind theme values */

  --font-sans: var(--font-sans);
  --font-mono: var(--font-mono);

  --radius-sm: var(--radius-sm);
  --radius-md: var(--radius-md);
  --radius-lg: var(--radius-lg);
  --radius-pill: var(--radius-pill);

  --shadow-sm: var(--shadow-sm);
  --shadow-md: var(--shadow-md);
  --shadow-lg: var(--shadow-lg);
  --shadow-glow: var(--shadow-glow);
}
```

This means: `theme/default.css` **remains the single source of truth**. Tailwind simply references our existing vars — no duplication.

---

## 2. Design Language: Next-Gen Obsidian

### 2.1 Visual Identity Principles

**Obsidian Dark** — elevated, not gothic. Think Linear's clarity + Vercel's precision.

| Principle | Implementation |
|-----------|---------------|
| **Depth through surface layers** | `surface-base` → `surface-raised` → `surface-overlay` — never flat stacking |
| **Accent economy** | Blue (`--color-accent`) only on interactive/active state. Status colors are data, not decoration |
| **Information density** | 14px base, tight spacing, no padding waste — data visible at a glance |
| **Motion with purpose** | Entrances: `ease-out` 150ms. Exits: `ease-in` 100ms. No decorative loops |
| **Glass elevation** | Modals/drawers: backdrop-blur + translucent surface, not opaque |
| **Icon consistency** | Lucide React throughout — no emoji, no mixed icon families |
| **Focus rings** | 2px `--color-accent` ring, 2px offset. Always visible |

### 2.2 Component Visual Targets

- **Nav rail**: 220px, fixed, `surface-nav` bg, pill-shaped active state with accent glow
- **Command palette**: Full-screen dim backdrop, centered frosted card, instant `Cmd+K`
- **Tables**: Sticky headers, alternating row hover, inline sort indicators, column resize
- **Kanban**: Smooth drag ghost with drop zone highlight, column scroll
- **Modals**: `backdrop-blur-md`, `surface-overlay` bg at 90% opacity, spring entrance animation
- **Toast**: Bottom-right stack, auto-dismiss, success/error/info variants

---

## 3. Dependency Additions Required

### 3.1 New Packages to Install

```bash
# Command palette
bun add cmdk

# Data tables
bun add @tanstack/react-table

# Virtual scrolling
bun add @tanstack/react-virtual

# Drag and drop
bun add @dnd-kit/core @dnd-kit/sortable @dnd-kit/utilities

# Toast notifications
bun add sonner

# Motion/animations
bun add motion

# Hotkeys
bun add hotkeys-js
# or: bun add @github/hotkey

# Radix additions (beyond what's installed)
bun add @radix-ui/react-context-menu
bun add @radix-ui/react-tooltip
bun add @radix-ui/react-dropdown-menu
bun add @radix-ui/react-separator
bun add @radix-ui/react-toggle
bun add @radix-ui/react-select
```

### 3.2 shadcn Components to Initialize

```bash
# From apps/desktop/
npx shadcn@latest init  # Select: TypeScript, CSS variables, no Tailwind config file (v4 mode)

# Then add components:
npx shadcn@latest add button
npx shadcn@latest add badge
npx shadcn@latest add dialog
npx shadcn@latest add tabs
npx shadcn@latest add scroll-area
npx shadcn@latest add popover
npx shadcn@latest add card
npx shadcn@latest add progress
npx shadcn@latest add avatar
npx shadcn@latest add tooltip
npx shadcn@latest add dropdown-menu
npx shadcn@latest add context-menu
npx shadcn@latest add separator
npx shadcn@latest add input
npx shadcn@latest add textarea
npx shadcn@latest add select
npx shadcn@latest add toggle
npx shadcn@latest add command  # includes cmdk under the hood
npx shadcn@latest add sonner   # wraps sonner toast
```

---

## 4. Sprint Plan

### Sprint 1: Foundation (Week 1)
**Shippable:** App builds with Tailwind v4 active, `cn()` utility live, shadcn initialized, command palette wired to `Cmd+K`

**Tasks:**
1. Add `@import "tailwindcss"` to `src/index.css`
2. Add `@theme {}` block bridging all semantic CSS vars to Tailwind theme (see §1.3)
3. Create `src/lib/utils.ts` — `cn()` = `clsx` + `tailwind-merge`
4. Create `src/lib/motion.ts` — shared animation variants (fadeIn, slideUp, scaleIn)
5. Run `npx shadcn@latest init` in Tailwind v4 mode
6. Install all shadcn components listed in §3.2
7. Install additional packages from §3.1
8. Build `CommandPalette` component wrapping shadcn `Command` + `cmdk`
   - Wire to `Cmd+K` global hotkey
   - Actions: navigate routes, open modals, trigger work orders
9. Add `<Toaster>` (sonner) to App.tsx root
10. Verify: `bun run build` passes, no CSS conflicts

**Files Created:**
- `src/index.css` (modified — add Tailwind imports + `@theme`)
- `src/lib/utils.ts`
- `src/lib/motion.ts`
- `src/components/ui/*` (shadcn output)
- `src/components/CommandPalette.tsx`

---

### Sprint 2: Common Component Library (Week 2)
**Shippable:** `common.tsx` fully migrated. All shared primitives use shadcn/Radix/Tailwind.

**Migration Map:**

| Current | Replacement | Notes |
|---------|-------------|-------|
| `StatusPill` | shadcn `Badge` + CVA variants | Map all `StatusTone` values to variants |
| `PriorityPill` | shadcn `Badge` + CVA variants | URGENT/HIGH/NORMAL with color tokens |
| `CriticalPathBadge` | shadcn `Badge` + `--color-critical` | Orange variant |
| `PresenceDot` | Radix-free, Tailwind + CVA | Keep simple — 8px dot with status color |
| `AgentAvatar` | Radix `Avatar` | Add `PresenceDot` overlay |
| `CellProgress` | shadcn `Progress` | Thin variant, accent color |
| `StatCard` | shadcn `Card` + Tailwind | Minimal border, raised surface |
| `ClampText` | Tailwind `line-clamp-*` utilities | Remove custom component |
| `ExpandChevron` | Lucide `ChevronRight` + Tailwind `rotate-90` | Animate with `transition-transform` |
| `EmptyState` | Tailwind + Lucide icons | Replace Unicode/emoji icons |
| `SectionHeader` | Tailwind + shadcn `Separator` | Clean typography |
| `SlideDrawer` | Radix `Dialog` + custom slide positioning | `motion` for spring entrance |
| `Modal` | shadcn `Dialog` | Replace size classes with Tailwind |
| `ArtifactPreviewContent` | Keep logic, Tailwind layout | Prose styling for markdown |
| `PreviewDialog` | shadcn `Dialog` wrapping artifact content | |

**Design rules for this sprint:**
- All variants defined with CVA in component files
- No hardcoded hex values — only `var(--*)` or Tailwind theme tokens
- All interactive elements have `:focus-visible` ring using `--color-accent`
- Replace every emoji/Unicode icon with Lucide equivalent

---

### Sprint 3: Data Layer Components (Week 3)
**Shippable:** Reusable table system, drag-drop board, virtual list — powering the hardest routes.

**Tasks:**
1. Build `DataTable<T>` wrapper around TanStack Table v8
   - Column definitions with type-safe `accessorFn`
   - Sortable columns with `SortIcon` (Lucide)
   - Row selection (checkbox column)
   - Sticky header
   - Empty state slot
   - Tailwind styling throughout
2. Build `VirtualList<T>` wrapper around TanStack Virtual
   - Overscan 5 items
   - Smooth scroll behavior
3. Build `KanbanBoard` + `KanbanColumn` + `KanbanCard` using `@dnd-kit`
   - Drag ghost: 90% opacity, 2px accent border
   - Drop zone: accent border highlight + subtle bg
   - Smooth column scroll on drag-over edge
4. Build `ContextMenu` wrapper (Radix ContextMenu)
   - Standard items: Open, Copy ID, Copy Link, Separator, Status Change, Delete
   - Used on every row/card surface
5. Build global `Tooltip` wrapper (Radix Tooltip + Tailwind)

---

### Sprint 4: Route Migration — Inbox, Projects, Work Orders (Week 4)
**Shippable:** Three primary routes fully migrated. All BEM classes gone from these files.

**InboxRoute** (medium complexity)
- Replace filter bar BEM classes with Tailwind flex + gap utilities
- `InboxItem` rows: Tailwind, hover state, unread indicator
- Status/priority pills: migrated components from Sprint 2
- Empty state: migrated component

**ProjectsRoute** (high complexity — 4 view modes)
- `list` view → `DataTable<ProjectRowModel>` from Sprint 3
- `board` view → `KanbanBoard` from Sprint 3
- `timeline` view — keep existing SVG/canvas, Tailwind wrapper
- `dependencies` view — keep existing graph, Tailwind wrapper
- `ProjectWorkspacePanel` — shadcn `Tabs` + Tailwind layout

**WorkOrdersRoute** (highest complexity)
- Table view → `DataTable<SwoRecord>` (sortable: status, priority, assignee, project)
- `VirtualList` for large queues (100+ SWOs)
- Filter/sort controls → Tailwind flex toolbar
- `WorkOrderDetail` panel → shadcn `Tabs`
- `ScheduleRwoModal` → shadcn `Dialog` + shadcn form components (Input, Select, Textarea)
- `WorkOrderRecoveryModal` → shadcn `Dialog`

---

### Sprint 5: Route Migration — Agents, Artifacts, Overview, Settings (Week 5)
**Shippable:** All remaining routes migrated. App shell migrated.

**AgentsRoute** (highest complexity — 7 tabs per agent)
- Agent list: `DataTable` or `VirtualList` depending on count
- Agent detail: shadcn `Tabs` (overview, charter, skills, tools, files, history, memory)
- `AgentAvatar` with presence: Sprint 2 component
- `NewAgentModal` → shadcn `Dialog` + multi-step form
- Pixel office view: Tailwind wrapper only (keep canvas logic)

**ArtifactsRoute**
- Grid/list toggle: Tailwind grid utilities
- `ArtifactCard`: shadcn `Card` + Tailwind
- `PreviewDialog`: Sprint 2 component

**OverviewRoute**
- `StatCard` grid: Sprint 2 components
- Timeline feed: Tailwind list + `ProjectTimelineFeed`
- `SairgentPanel`: shadcn `Tabs` + Tailwind

**SettingsRoute**
- shadcn `Tabs` for settings sections
- Form inputs: shadcn Input, Select, Toggle
- Section cards: shadcn `Card`

**App Shell (App.tsx)**
- Nav rail: Tailwind `flex flex-col`, active state = `bg-[var(--color-accent-glow)] text-[var(--color-accent-text)]` pill
- Topbar: Tailwind flex, shadcn `Separator`
- Replace all BEM shell classes with Tailwind utilities

---

### Sprint 6: Polish, Accessibility, Testing (Week 6)
**Shippable:** Production-ready. WCAG 2.1 AA. Performance within targets. `index.css` archived.

**Motion & Animation Polish**
- Confirm all route transitions use `motion` (fade + slide, 150ms)
- Modal/drawer entrances: spring physics
- Toast stack: animated push/dismiss
- Kanban drag: momentum ghost
- Command palette: instant open, subtle scale entrance

**Accessibility Audit**
- All Radix primitives: verify ARIA labels on every instance
- Focus trapping: modals, drawers, command palette
- Keyboard nav: tab order through all routes
- Screen reader: test with VoiceOver on Obsidian dark theme
- Color contrast: all text against surface backgrounds (WCAG AA minimum)
- Skip links: main content, nav rail
- Live regions: inbox unread count, SWO status changes

**Performance**
- Tailwind v4 generates minimal CSS — verify bundle size
- TanStack Virtual active on any list > 50 items
- Code split: lazy-load route components
- Image/icon: Lucide tree-shakes automatically

**Final Cleanup**
- Delete `src/index.css` BEM class blocks (keep reset + root vars)
- Archive legacy CSS: `99_archive/legacy-index-css-[date].css`
- `theme/default.css` remains unchanged (still source of truth)
- ESLint pass: no inline styles, no BEM class names remaining

**Success Metrics:**

| Metric | Target |
|--------|--------|
| Components using Tailwind utilities | 100% |
| Modals using Radix Dialog | 100% |
| Icons using Lucide React | 100% |
| WCAG 2.1 AA contrast | Pass |
| Bundle size delta | ≤ +5% |
| LCP (route load) | < 200ms |
| Cmd+K command palette | < 50ms open |
| Lighthouse accessibility | ≥ 95 |

---

## 5. Component Architecture Standards

### 5.1 CVA Pattern for Variants

```tsx
// Pattern for all variant-bearing components
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badge = cva(
  "inline-flex items-center gap-1 rounded-[var(--radius-sm)] text-[11px] font-medium px-2 py-0.5",
  {
    variants: {
      tone: {
        neutral:  "bg-[var(--color-neutral-bg)]  text-[var(--color-neutral)]",
        working:  "bg-[var(--color-active-bg)]   text-[var(--color-active)]",
        blocked:  "bg-[var(--color-blocked-bg)]  text-[var(--color-blocked)]",
        review:   "bg-[var(--color-review-bg)]   text-[var(--color-review)]",
        complete: "bg-[var(--color-complete-bg)] text-[var(--color-complete)]",
        critical: "bg-[var(--color-critical-bg)] text-[var(--color-critical)]",
      },
    },
    defaultVariants: { tone: "neutral" },
  }
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof badge> {}

export function StatusBadge({ tone, className, ...props }: BadgeProps) {
  return <span className={cn(badge({ tone }), className)} {...props} />;
}
```

### 5.2 Motion Variants (from `src/lib/motion.ts`)

```ts
export const fadeIn = { hidden: { opacity: 0 }, visible: { opacity: 1, transition: { duration: 0.15, ease: [0.22, 1, 0.36, 1] } } };
export const slideUp = { hidden: { opacity: 0, y: 8 }, visible: { opacity: 1, y: 0, transition: { duration: 0.15, ease: [0.22, 1, 0.36, 1] } } };
export const scaleIn = { hidden: { opacity: 0, scale: 0.96 }, visible: { opacity: 1, scale: 1, transition: { duration: 0.12, ease: [0.22, 1, 0.36, 1] } } };
```

### 5.3 `cn()` Utility

```ts
// src/lib/utils.ts
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
```

### 5.4 No Hardcoded Values Rule

- Never hardcode hex colors in className strings
- Always use `var(--*)` CSS variables or Tailwind theme tokens
- Theme tokens in `@theme {}` map to CSS vars — both spellings work

---

## 6. File Structure (Post-Migration)

```
apps/desktop/src/
  index.css                    ← Tailwind @import + @theme + reset only (BEM gone)
  theme/default.css            ← PRESERVED: Primitive + semantic token source of truth
  lib/
    utils.ts                   ← cn() utility
    motion.ts                  ← Shared animation variants
  components/
    ui/                        ← shadcn auto-generated components
    CommandPalette.tsx          ← Cmd+K global command palette
    DataTable.tsx              ← TanStack Table v8 wrapper
    VirtualList.tsx            ← TanStack Virtual wrapper
    KanbanBoard.tsx            ← dnd-kit kanban system
  ui/                          ← Route surfaces (migrated in-place)
    common.tsx                 ← Migrated: all primitives use shadcn/Tailwind
    AgentsRoute.tsx            ← etc.
```

---

## 7. Roles & Responsibilities

| Role | Responsibility |
|------|---------------|
| **Jimmy (CDO)** | Component design quality, visual spec enforcement, CVA patterns, accessibility |
| **Felicity (CTO)** | React architecture review, TypeScript patterns, TanStack integration |
| **Kryptonite (CISO)** | Sprint 1: dependency audit + CSP review; Sprint 6: XSS review of new components |
| **Oracle (CPO)** | Sprint 1 consult: confirm component architecture supports future SaaS surfaces |

---

## 8. Security Checkpoints (Kryptonite)

| Sprint | Review |
|--------|--------|
| 1 | Audit all new deps: cmdk, dnd-kit, sonner, motion, TanStack packages |
| 1 | CSP update for Tailwind's inline style injections (v4 uses `<style>` injection) |
| 4 | XSS audit: `ArtifactPreviewContent` markdown rendering (DOMPurify in place — verify) |
| 6 | Final XSS pass on all new shadcn/Radix components with dynamic content |

---

## 9. Risk Register

| Risk | Likelihood | Mitigation |
|------|-----------|-----------|
| Tailwind v4 + shadcn v4 init incompatibility | Medium | Test `npx shadcn@latest init` first in Sprint 1 before any component work |
| Token naming conflicts (Tailwind `@theme` vs CSS vars) | Low | Namespace Tailwind tokens as `color-surface-*`, `color-text-*` to avoid collisions |
| `motion` library bundle size | Low | Tree-shake; only import used exports |
| dnd-kit a11y gaps | Medium | Use built-in keyboard support; test with screen reader in Sprint 6 |
| Route regression during migration | Medium | Test each route after its sprint before moving on |
| CSS specificity conflicts during transition | High | Migrate component-by-component; run `bun run build` after each file |

---

## 10. Next Steps

1. ✅ Plan revised and approved
2. ⏳ Jimmy: review design language spec (§2), confirm visual targets
3. ⏳ Kryptonite: pre-flight dependency audit before Sprint 1
4. ⏳ Oracle: confirm component architecture covers SaaS surfaces (§5 standards)
5. ⏳ Capture baseline metrics: `bun run build` bundle size, Lighthouse score
6. ⏳ Sprint 1: Foundation — `bun add` packages, Tailwind v4 init, command palette

---

**Plan Status:** REVISED — PENDING REVIEW
**Migration Approach:** 6 sprints, desktop-first
**Design Vision:** Linear/Vercel-caliber Obsidian dark — keyboard-first, data-dense, fluid
**Skill Owner:** Jimmy (CDO) extended
**SaaS Strategy:** Desktop-first; component architecture SaaS-ready by design
**Risk Level:** Medium-Low (foundation sprint de-risks Tailwind v4 compatibility before component work)
