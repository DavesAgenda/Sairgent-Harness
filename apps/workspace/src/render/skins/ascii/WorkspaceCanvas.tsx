import { useRef } from 'react';
import type { WorkspaceWorld } from '../../../types';
import { AgentDesk } from './AgentDesk';
import { BenchRow } from './BenchRow';
import { TubeOverlay } from './TubeOverlay';

interface WorkspaceCanvasProps {
  world: WorkspaceWorld;
  onDeskClick: (agentId: string) => void;
}

/** Max occupied row/col so we can size the grid precisely. */
function gridDimensions(desks: WorkspaceWorld['desks']): { rows: number; cols: number } {
  if (desks.length === 0) return { rows: 1, cols: 6 };
  const maxRow = Math.max(...desks.map((d) => d.gridRow));
  const maxCol = Math.max(...desks.map((d) => d.gridCol));
  return { rows: maxRow + 1, cols: Math.max(maxCol + 1, 6) };
}

export function WorkspaceCanvas({ world, onDeskClick }: WorkspaceCanvasProps) {
  const gridRef = useRef<HTMLDivElement>(null);
  const { desks, tubes, bench } = world;

  const hasActive = desks.length > 0;
  const { rows, cols } = gridDimensions(desks);

  return (
    <div className="flex flex-col w-full h-full font-mono">
      {/* Active desk grid + tube overlay — scrollable */}
      <div className="flex-1 overflow-auto">
        <div
          ref={gridRef}
          className="relative w-full"
          style={{
            display: 'grid',
            gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
            gridTemplateRows: `repeat(${rows}, auto)`,
            gap: '1.5rem',
            padding: '1.5rem',
            minHeight: hasActive ? undefined : '12rem',
          }}
        >
          {/* Empty state */}
          {!hasActive && (
            <div
              className="col-span-full flex items-center justify-center text-neutral-600 text-xs tracking-widest uppercase"
              style={{ gridColumn: `1 / span ${cols}` }}
            >
              Workspace idle — submit a job to begin
            </div>
          )}

          {/* Desk cards positioned by grid row/col */}
          {desks.map((desk) => (
            <div
              key={desk.agentId}
              style={{
                gridRow:    desk.gridRow + 1,
                gridColumn: desk.gridCol + 1,
                justifySelf: 'center',
              }}
            >
              <AgentDesk
                desk={desk}
                onDeskClick={onDeskClick}
                liveActivity={world.agentLiveActivity?.[desk.agentId]}
              />
            </div>
          ))}

          {/* Tube overlay sits over the grid */}
          {hasActive && tubes.length > 0 && (
            <TubeOverlay tubes={tubes} containerRef={gridRef} />
          )}
        </div>
      </div>

      {/* Bench row pinned to bottom */}
      {bench.length > 0 && (
        <div
          className="px-6 py-3 shrink-0"
          style={{ borderTop: '1px solid var(--ws-border)' }}
        >
          <BenchRow agents={bench} onDeskClick={onDeskClick} />
        </div>
      )}
    </div>
  );
}
