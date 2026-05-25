# Sairgent Visual Workspace

Spatial, game-inspired prototype showing Sairgent's agent delegation as an interactive workspace. Agents are workstation desks on a grid; work flows between them via animated pneumatic tubes with capsules.

## Quick Start

```bash
cd apps/workspace
bun install
bun run dev        # http://localhost:5173
```

## What It Does

Submit a job and watch delegation cascade through the agent org:

1. **Perry (COO)** receives the job and assesses it
2. He delegates sub-tasks to specialists (Lois, Lex, Felicity, etc.)
3. Specialists may sub-delegate further (Lois -> Stacker)
4. Animated tube capsules show work flowing between desks
5. Completions flow back up the chain
6. Deliverables land in the inbox tray

## Architecture

```
src/
  types.ts              Shared contracts (Agent, SWO, Signal, World)
  sim/                  Mock kernel layer
    mockRoster.ts       15-agent org tree
    mockBus.ts          Event bus (Bus interface — seam for real kernel)
    mockScenarios.ts    Scripted delegation flows with timing
    signalRecorder.ts   Record signals for replay testing (stub)
    replayBus.ts        Replay recorded signals (stub)
  world/                Pure state derivation from signals
    layoutEngine.ts     Roster + SWOs -> grid positions
    tubePathComputer.ts Desk positions -> tube connections
    useWorkspaceState.ts Master hook: bus -> WorkspaceWorld
  render/               Visual components (ASCII skin)
    AgentDesk.tsx       Single workstation box
    BenchRow.tsx        Idle agents row
    WorkspaceCanvas.tsx Grid container + tube overlay
    TubeOverlay.tsx     SVG tube paths between desks
    TubeCapsule.tsx     Animated capsule on tube path
    InboxTray.tsx       Bottom deliverables strip
  chrome/               UI chrome
    Header.tsx          Top bar with submit + inbox
    SubmitJobDialog.tsx Job submission form
    AgentInspector.tsx  Agent profile drawer
    ArtifactViewer.tsx  Deliverable preview
    DevToolbar.tsx      Scenario trigger buttons
```

## Dev Toolbar Scenarios

Click buttons in the bottom-left to trigger:

- **Happy Path** — Full delegation chain: Perry -> Lois + Lex -> Stacker -> completion
- **Blocked Path** — Felicity hits a blocker, escalates, resolves
- **Parallel Burst** — 3 jobs submitted 500ms apart
- **Reset** — Clear workspace to idle state

## Skin Architecture

The `world/` layer outputs abstract `WorkspaceWorld` state. The `render/` layer consumes it visually. Swapping to a pixel-art or 3D skin means replacing `render/` components while `world/` and `chrome/` stay unchanged.

The `Bus` interface is the kernel seam. Replace `sim/mockBus.ts` with a Tauri IPC bridge emitting the same `RuntimeSignal` shapes and the workspace connects to the real kernel with zero changes to `world/` or `render/`.

## Tech Stack

React 19 | Vite 6 | Tailwind CSS 4 | Motion (framer-motion) | Radix UI | Lucide React

## Commands

```bash
bun run dev          # Dev server
bun run build        # TypeScript check + production build
bun run test         # 90 tests (unit + integration + component)
bun run test:watch   # Watch mode
```
