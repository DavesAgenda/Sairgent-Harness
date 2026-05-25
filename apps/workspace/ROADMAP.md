# Visual Workspace Roadmap

## Phase 1: ASCII Terminal Skin (DONE)

The shipped POC. Dark terminal aesthetic, Unicode box-drawing characters, SVG tubes with Motion capsule animations. 15-agent roster with 3 mock scenarios.

**Status:** Shipped. 42 files, 90 tests, `bun run dev` works.

---

## Phase 2: Wire to Real Kernel

Connect the workspace to the real Sairgent kernel via the `Bus` interface seam. No visual changes — same ASCII skin, real data.

| Issue | What | Priority |
|-------|------|----------|
| CHA-287 | Tauri IPC bridge (`tauriBus.ts` implementing `Bus`) | High |
| CHA-291 | Real job submission via `swo.create` runtime command | High |
| CHA-290 | Live progress streaming + agent activity log sidebar | High |
| CHA-288 | Record/replay testing with real kernel signals | Medium |

**Gate:** Phase 2 must work before any new skin. Skins are cosmetic; the data pipeline is structural.

---

## Phase 3: Skin System + Alternate Skins

All skins consume the same `WorkspaceWorld` contract from `world/`. Only `render/` components change. Skins live in `render/skins/{name}/` and implement the `WorkspaceSkin` interface.

### Skin Loader Architecture

```typescript
export interface WorkspaceSkin {
  WorkspaceCanvas: React.FC<{ world: WorkspaceWorld; onDeskClick: (id: string) => void }>;
  AgentDesk: React.FC<{ desk: DeskState; onClick: () => void }>;
  TubeOverlay: React.FC<{ tubes: TubeState[] }>;
  InboxTray: React.FC<{ items: InboxItem[]; onItemClick: (id: string) => void }>;
}

// Lazy-load skins
const skins = {
  ascii: () => import('./skins/ascii'),
  'pixel-office': () => import('./skins/pixel-office'),
  factory: () => import('./skins/factory'),
  // ...
};
```

The `chrome/` layer (drawers, dialogs, forms) stays constant across all skins.

---

### Full Skin Catalog

Organized into tiers by implementation effort and new dependency requirements.

#### Tier 1: CSS-Only Skins (1-2 days each, zero new deps)

These skins change only styling, colors, and static assets. Same React components, same SVG tubes, same Motion animations. The fastest to ship.

| Skin | Aesthetic | Agent Style | Flow Style | Audience |
|------|-----------|-------------|------------|----------|
| **emoji** | Pure Unicode/emoji | `🧑‍💻 👩‍🔬 🧙` emoji characters | Arrow chains `➡️ ⬇️ ⬆️` | Lightweight / accessible |
| **win95** | Windows 95 chrome | Desktop icons in draggable windows | Flying folder animations | Nostalgia / humor |
| **business** | Clean, minimal, corporate | Avatar circles with status dots | Kanban-style flow lines | Enterprise / serious users |
| **command-deck** | "Starship bridge" C2 | Status panels with metrics | Cyan data-flow lines + HUD | Enterprise / cinematic |

**Why these are easy:**
- Emoji skin: literally swap the `icon` field rendering and tube stroke styles. An hour of work.
- Win95: CSS window chrome (title bar, grey beveled borders) + folder icon sprites. No layout changes.
- Business: Strip the terminal aesthetic, add Inter font, muted blues/greys, rounded cards. Styling only.
- Command Deck: Deep Space Obsidian (`#05080F`) + Electric Cyan (`#00F0FF`), already has a design spec in `00_Context/Product/archive/ui_concepts.md`.

#### Tier 2: CSS Sprite Skins (2-4 days each, art assets needed)

These add pixel-art sprites via CSS `image-rendering: pixelated` and `background-position` stepping. No PixiJS, no canvas — pure CSS animation. Research confirms this handles 15-20 agents trivially.

| Skin | Aesthetic | Agent Style | Flow Style | Audience |
|------|-----------|-------------|------------|----------|
| **pixel-office** | 16-bit Stardew Valley | Pixel characters at desks | Pneumatic tubes with glass capsules | Primary product skin |
| **factory** | Factorio/Builderment | Machines with inputs | Conveyor belts with packages | Engineers / technical |
| **scifi** | Neon, holographic | Hologram avatars in pods | Energy beams / data streams | Futuristic aesthetic |
| **medieval** | Pixel castle/village | Knights, scribes, craftsmen | Carrier pigeons / horse messengers | Fantasy fans |
| **farm** | Stardew-style outdoors | Farmers at crop stations | Water irrigation channels | Casual / approachable |
| **desert** | Sand/oasis outpost | Nomads at tent stations | Camel caravans / sand pipes | Adventure aesthetic |
| **ocean** | Underwater research base | Divers at stations | Submarine message pods | Unique aesthetic |
| **orcs** | Warcraft pixel art | Peons, grunts, shamans | War drums / raven messengers | Gaming audience |

**Asset pipeline:** Piskel (browser, free) for prototypes → Aseprite ($20) for production. 32x32 base resolution, PNG spritesheets with JSON metadata. Idle (1px breathing) + working (context-specific) + walking (4-frame cycle).

**Recommended first:** `pixel-office` — it's the flagship product skin per the original vision.

#### Tier 3: Layout-Changing Skins (4-7 days each)

These change the spatial layout algorithm, not just the rendering. The `world/layoutEngine.ts` grows new layout modes, but the `WorkspaceWorld` contract stays unchanged.

| Skin | Layout Change | Effort Driver |
|------|---------------|---------------|
| **isometric-office** | Isometric coordinate transform | Iso math + 4-direction sprites |
| **floorplan** | Room-based layout with walls/doors | Floor plan generation algorithm |

#### Tier 4: Engine Skins (1-2 weeks)

These require PixiJS or another WebGL renderer. Only justified at 50+ agents or for particle/shader effects.

| Skin | Engine | What It Enables |
|------|--------|-----------------|
| **pixel-office-hd** | PixiJS | Particle effects, shader glow, 60fps sprites, ambient sound |

**Decision rule:** Stay in Tier 1-2 as long as possible. PixiJS adds ~200KB gzipped, concurrent mode gotchas, and React integration complexity. It's overkill for 15 agents.

---

## Recommended Build Sequence

```
Phase 1 (DONE)     Phase 2              Phase 3 — Tier 1          Phase 3 — Tier 2
ASCII Terminal  →  Real Kernel Wire  →  emoji (1 hr)           →  pixel-office (3 days)
                                        business (1 day)           factory (3 days)
                                        win95 (1 day)              medieval (4 days)
                                        command-deck (2 days)      [community skins...]
```

### Why Emoji Skin First

It's the absolute lowest effort skin and proves the skin-swap architecture works end-to-end. If we can hot-swap emoji ↔ ascii, the `WorkspaceSkin` interface is validated and every subsequent skin is just art + CSS.

### Phase 3 Polish (applies to all skins)

| Feature | Effort | Notes |
|---------|--------|-------|
| Skin selector in Settings | 1 day | Dropdown, persisted to localStorage |
| Ambient sound toggle | 2 days | Web Audio API: keyboard clicks, pneumatic whoosh, delivery ding |
| Day/night cycle | 1 day | Tied to real time, affects background tint |
| Completion particles | 1 day (CSS) or 3 days (PixiJS) | Sparks/glow on capsule arrival |
| Mini-map | 2 days | For large teams (15+ active agents) |

### Monetization Angle

Skins are a natural monetization surface:
- **Free:** ascii, emoji, business
- **Premium:** pixel-office, factory, medieval, ocean, orcs, etc.
- **Unlock via milestones:** Complete N jobs → unlock a skin
- **Community marketplace:** Users create and share skins (Tier 1-2 only, no engine dependency)

---

## Skin Architecture Invariant

Every skin MUST consume `WorkspaceWorld` and ONLY replace files in `render/skins/{name}/`. The `world/`, `sim/`, `chrome/`, and `App.tsx` layers are skin-agnostic. This is a hard rule — if a skin needs state changes, the state change goes in `world/` and all skins benefit.

```
                    ┌────────────────┐
                    │  sim/ (Bus)    │
                    └───────┬────────┘
                            │ RuntimeSignal
                    ┌───────▼────────┐
                    │  world/        │
                    │  (state engine)│
                    └───────┬────────┘
                            │ WorkspaceWorld
         ┌──────────────────┼──────────────────┐
         │                  │                  │
  ┌──────▼───┐      ┌──────▼───┐      ┌──────▼───┐
  │  ascii   │      │  emoji   │      │  pixel   │  ...
  │  render/ │      │  render/ │      │  render/ │
  └──────────┘      └──────────┘      └──────────┘
```

## "Factorio Brain" UX Principles (from research)

Five patterns drive the factory-sim satisfaction loop — all skins should honor these:

1. **Visible throughput** — Items constantly moving. Belt/tube fullness = utilization at a glance.
2. **Idle vs busy contrast** — Distinct idle/working animations. Motion clusters signal where work is happening.
3. **Accumulation** — Visible stockpiles, filling containers, growing output. Progress beyond numbers.
4. **Completion signals** — Clear visual/audio feedback on milestones. Capsule arrives with flash/bounce.
5. **Legible patterns** — Same agent, same position. Complex systems stay readable via repeating motifs.
