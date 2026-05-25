import type { WorkspaceWorld, InboxItem } from '../types';

/**
 * Contract every visual skin must satisfy.
 *
 * Skins are render-only -- they receive the canonical WorkspaceWorld
 * (produced by world/) and translate it into pixels.  They must NEVER
 * mutate world state or call into the bus.
 */
export interface WorkspaceSkin {
  /** Machine-readable identifier stored in localStorage. */
  id: string;
  /** Human-readable label shown in the skin picker. */
  name: string;
  /** One-liner for tooltips / settings panel. */
  description: string;
  /** Main canvas: desk grid + tube overlay + bench row. */
  WorkspaceCanvas: React.FC<{ world: WorkspaceWorld; onDeskClick: (id: string) => void }>;
  /** Bottom deliverables tray. */
  InboxTray: React.FC<{ items: InboxItem[]; onItemClick: (id: string) => void }>;
}
