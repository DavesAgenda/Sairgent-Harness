import type { DeskState } from '../../../types';

interface BenchRowProps {
  agents: DeskState[];
  onDeskClick: (agentId: string) => void;
}

export function BenchRow({ agents, onDeskClick }: BenchRowProps) {
  if (agents.length === 0) return null;

  return (
    <div className="mt-6 font-mono">
      {/* Section label */}
      <div className="text-[10px] uppercase tracking-[0.25em] text-neutral-600 mb-2 px-1">
        BENCH — {agents.length} agent{agents.length !== 1 ? 's' : ''} idle
      </div>

      {/* Bench divider */}
      <div className="text-neutral-700 text-[10px] mb-2 px-1 select-none">
        {'\u2500'.repeat(60)}
      </div>

      {/* Agent chips */}
      <div className="flex flex-wrap gap-2 px-1">
        {agents.map((desk) => (
          <button
            key={desk.agentId}
            data-agent-id={desk.agentId}
            onClick={() => onDeskClick(desk.agentId)}
            className={[
              'flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider',
              'border border-neutral-700/40 bg-neutral-900/40 rounded-sm px-2 py-1',
              'text-neutral-500 hover:text-neutral-300 hover:border-neutral-600/60',
              'transition-colors duration-150 cursor-pointer focus:outline-none',
              'focus:ring-1 focus:ring-green-500/40',
            ].join(' ')}
          >
            <span className="text-sm leading-none">{desk.icon}</span>
            <span>{desk.name}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
