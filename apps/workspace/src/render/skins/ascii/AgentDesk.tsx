import { useState, useEffect } from 'react';
import { motion } from 'motion/react';
import type { DeskState } from '../../../types';

interface AgentDeskProps {
  desk: DeskState;
  onDeskClick: (agentId: string) => void;
  liveActivity?: { text: string; updatedAt: number };
}

const PRESENCE_STYLES: Record<DeskState['presence'], string> = {
  COMPUTING: 'bg-blue-950/60 border-blue-700/50',
  READY:     'bg-green-950/60 border-green-700/50',
  IDLE:      'bg-neutral-900/60 border-neutral-700/40',
  STALE:     'bg-neutral-900/60 border-neutral-700/40',
  OFFLINE:   'bg-red-950/40 border-red-900/40',
};

const PRESENCE_LABEL_COLOR: Record<DeskState['presence'], string> = {
  COMPUTING: 'text-blue-400',
  READY:     'text-green-400',
  IDLE:      'text-neutral-500',
  STALE:     'text-neutral-500',
  OFFLINE:   'text-red-500',
};

const BRAILLE_FRAMES = ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];
const STALE_THRESHOLD_MS = 5000;

function taskLabel(desk: DeskState): string {
  if (desk.statusText) return desk.statusText;
  if (desk.currentTask) return desk.currentTask;
  if (desk.presence === 'OFFLINE') return 'Offline';
  if (desk.presence === 'IDLE') return 'Idle';
  if (desk.presence === 'COMPUTING') return 'Processing...';
  return 'Ready';
}

function useSpinnerFrame(active: boolean): string {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => {
      setFrame((f) => (f + 1) % BRAILLE_FRAMES.length);
    }, 80);
    return () => clearInterval(id);
  }, [active]);
  return BRAILLE_FRAMES[frame]!;
}

export function AgentDesk({ desk, onDeskClick, liveActivity }: AgentDeskProps) {
  const isComputing = desk.presence === 'COMPUTING';
  const spinnerFrame = useSpinnerFrame(isComputing);

  // Only show live text if it's recent enough and non-empty
  const now = Date.now();
  const isLiveFresh =
    liveActivity != null &&
    liveActivity.text.length > 0 &&
    now - liveActivity.updatedAt < STALE_THRESHOLD_MS;

  // Trim to last ~100 chars for display
  const displayText = isLiveFresh
    ? liveActivity!.text.slice(-100).trimStart()
    : null;

  return (
    <motion.div
      data-testid="agent-desk"
      data-agent-id={desk.agentId}
      role="button"
      tabIndex={0}
      onClick={() => onDeskClick(desk.agentId)}
      onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && onDeskClick(desk.agentId)}
      className={[
        'relative font-mono text-xs select-none cursor-pointer',
        'border rounded-sm px-0 py-0 w-40 min-h-[7.5rem]',
        'transition-colors duration-300 focus:outline-none focus:ring-1 focus:ring-green-500/50',
        PRESENCE_STYLES[desk.presence],
      ].join(' ')}
      animate={isComputing ? { boxShadow: ['0 0 0px #1d4ed860', '0 0 8px #3b82f680', '0 0 0px #1d4ed860'] } : { boxShadow: 'none' }}
      transition={isComputing ? { duration: 2, repeat: Infinity, ease: 'easeInOut' } : {}}
    >
      {/* Top border row */}
      <div className="text-neutral-600 px-1 pt-1 leading-none select-none">
        {'\u250C' + '\u2500'.repeat(13) + '\u2510'}
      </div>

      {/* Icon + name */}
      <div className="flex items-baseline gap-1 px-2 text-green-300 font-bold leading-snug">
        <span className="text-base leading-none">{desk.icon}</span>
        <span className="truncate uppercase tracking-widest text-[10px]">{desk.name}</span>
      </div>

      {/* Role */}
      <div className={`px-2 text-[10px] uppercase tracking-wider leading-snug ${PRESENCE_LABEL_COLOR[desk.presence]}`}>
        {desk.role}
      </div>

      {/* Spinner + live streaming text (replaces fake progress bar) */}
      {isComputing && (
        <div className="px-2 mt-0.5 flex items-start gap-1 min-h-[2.5rem]">
          <span
            className="text-blue-400 shrink-0 leading-tight"
            style={{ fontSize: '10px' }}
            aria-hidden="true"
          >
            {spinnerFrame}
          </span>
          {displayText ? (
            <motion.span
              key={displayText.slice(-20)}
              initial={{ opacity: 0.4 }}
              animate={{ opacity: [0.4, 1, 0.7, 1] }}
              transition={{ duration: 0.8, ease: 'easeInOut' }}
              className="text-[10px] leading-tight overflow-hidden"
              style={{
                color: 'rgb(74 222 128 / 0.7)',
                display: '-webkit-box',
                WebkitLineClamp: 3,
                WebkitBoxOrient: 'vertical',
                wordBreak: 'break-all',
              }}
            >
              {displayText}
            </motion.span>
          ) : (
            <span
              className="text-[10px] leading-tight text-neutral-600 italic"
            >
              {taskLabel(desk)}
            </span>
          )}
        </div>
      )}

      {/* Current task / status text (shown when not computing) */}
      {!isComputing && (
        <div className="px-2 pt-0.5 pb-1 text-[10px] text-neutral-400 leading-snug truncate">
          {taskLabel(desk)}
        </div>
      )}

      {/* Bottom border row */}
      <div className="text-neutral-600 px-1 pb-1 leading-none select-none">
        {'\u2514' + '\u2500'.repeat(13) + '\u2518'}
      </div>
    </motion.div>
  );
}
