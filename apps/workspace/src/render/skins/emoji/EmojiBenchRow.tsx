import type { DeskState } from '../../../types';
import { emojiIcon } from './agentIcons';

interface EmojiBenchRowProps {
  agents: DeskState[];
  onDeskClick: (agentId: string) => void;
}

export function EmojiBenchRow({ agents, onDeskClick }: EmojiBenchRowProps) {
  if (agents.length === 0) return null;

  return (
    <div className="mt-4">
      <div className="text-xs uppercase tracking-widest text-gray-400 mb-2 px-1">
        Bench -- {agents.length} agent{agents.length !== 1 ? 's' : ''} idle
      </div>

      <div className="flex flex-wrap gap-2 px-1">
        {agents.map((desk) => (
          <button
            key={desk.agentId}
            data-agent-id={desk.agentId}
            onClick={() => onDeskClick(desk.agentId)}
            className={[
              'flex items-center gap-1.5 text-xs',
              'border border-gray-200 bg-white rounded-lg px-3 py-1.5',
              'text-gray-500 hover:text-gray-700 hover:border-gray-400 hover:shadow-sm',
              'transition-all duration-150 cursor-pointer focus:outline-none',
              'focus:ring-2 focus:ring-blue-300',
            ].join(' ')}
          >
            <span className="text-base leading-none">{emojiIcon(desk.name)}</span>
            <span className="uppercase tracking-wide">{desk.name}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
