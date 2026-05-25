import { useRef } from 'react';
import type { WorkspaceWorld } from '../../../types';
import { EmojiAgentDesk } from './EmojiAgentDesk';
import { EmojiBenchRow } from './EmojiBenchRow';
import { EmojiTubeOverlay } from './EmojiTubeOverlay';

interface Props {
  world: WorkspaceWorld;
  onDeskClick: (agentId: string) => void;
}

function gridDimensions(desks: WorkspaceWorld['desks']): { rows: number; cols: number } {
  if (desks.length === 0) return { rows: 1, cols: 6 };
  const maxRow = Math.max(...desks.map((d) => d.gridRow));
  const maxCol = Math.max(...desks.map((d) => d.gridCol));
  return { rows: maxRow + 1, cols: Math.max(maxCol + 1, 6) };
}

export function EmojiWorkspaceCanvas({ world, onDeskClick }: Props) {
  const gridRef = useRef<HTMLDivElement>(null);
  const { desks, tubes, bench } = world;

  const hasActive = desks.length > 0;
  const { rows, cols } = gridDimensions(desks);

  return (
    <div className="flex flex-col w-full h-full">
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
          {!hasActive && (
            <div
              className="col-span-full flex items-center justify-center text-gray-400 text-sm tracking-widest"
              style={{ gridColumn: `1 / span ${cols}` }}
            >
              Workspace idle -- submit a job to begin
            </div>
          )}

          {desks.map((desk) => (
            <div
              key={desk.agentId}
              style={{
                gridRow:    desk.gridRow + 1,
                gridColumn: desk.gridCol + 1,
                justifySelf: 'center',
              }}
            >
              <EmojiAgentDesk
                desk={desk}
                onDeskClick={onDeskClick}
                liveActivity={world.agentLiveActivity?.[desk.agentId]}
              />
            </div>
          ))}

          {hasActive && tubes.length > 0 && (
            <EmojiTubeOverlay tubes={tubes} containerRef={gridRef} />
          )}
        </div>
      </div>

      {bench.length > 0 && (
        <div
          className="px-6 py-3 shrink-0"
          style={{ borderTop: '1px solid var(--ws-border)' }}
        >
          <EmojiBenchRow agents={bench} onDeskClick={onDeskClick} />
        </div>
      )}
    </div>
  );
}
