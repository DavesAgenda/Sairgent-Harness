import { useState, useEffect } from 'react';
import type { DeskState } from '../../../types';
import { emojiIcon } from './agentIcons';

interface EmojiAgentDeskProps {
  desk: DeskState;
  onDeskClick: (agentId: string) => void;
  liveActivity?: { text: string; updatedAt: number };
}

const PRESENCE_BG: Record<DeskState['presence'], string> = {
  COMPUTING: 'bg-blue-100 border-blue-300',
  READY:     'bg-green-100 border-green-300',
  IDLE:      'bg-gray-100 border-gray-300',
  STALE:     'bg-gray-100 border-gray-300',
  OFFLINE:   'bg-red-100 border-red-300',
};

const PRESENCE_TEXT: Record<DeskState['presence'], string> = {
  COMPUTING: 'text-blue-700',
  READY:     'text-green-700',
  IDLE:      'text-gray-500',
  STALE:     'text-gray-500',
  OFFLINE:   'text-red-600',
};

const SPINNER_FRAMES = ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];
const STALE_THRESHOLD_MS = 5000;

function taskLabel(desk: DeskState): string {
  if (desk.presence === 'OFFLINE') return 'Offline';
  if (desk.presence === 'IDLE') return 'Idle';
  if (desk.currentTask) return desk.currentTask;
  return 'Ready';
}

function useSpinnerFrame(active: boolean): string {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => {
      setFrame((f) => (f + 1) % SPINNER_FRAMES.length);
    }, 80);
    return () => clearInterval(id);
  }, [active]);
  return SPINNER_FRAMES[frame]!;
}

export function EmojiAgentDesk({ desk, onDeskClick, liveActivity }: EmojiAgentDeskProps) {
  const isComputing = desk.presence === 'COMPUTING';
  const spinnerFrame = useSpinnerFrame(isComputing);
  const icon = emojiIcon(desk.name);

  const now = Date.now();
  const isLiveFresh =
    liveActivity != null &&
    liveActivity.text.length > 0 &&
    now - liveActivity.updatedAt < STALE_THRESHOLD_MS;

  const displayText = isLiveFresh
    ? liveActivity!.text.slice(-100).trimStart()
    : null;

  return (
    <div
      data-testid="agent-desk"
      data-agent-id={desk.agentId}
      role="button"
      tabIndex={0}
      onClick={() => onDeskClick(desk.agentId)}
      onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && onDeskClick(desk.agentId)}
      className={[
        'relative text-sm select-none cursor-pointer',
        'border-2 rounded-xl px-3 py-2 w-44 min-h-[7.5rem]',
        'transition-all duration-200 hover:shadow-md focus:outline-none focus:ring-2 focus:ring-blue-400/60',
        PRESENCE_BG[desk.presence],
      ].join(' ')}
    >
      {/* Icon + name */}
      <div className="flex items-center gap-2 mb-1">
        <span className="text-2xl leading-none">{icon}</span>
        <span className="font-semibold text-gray-800 uppercase tracking-wide text-xs truncate">
          {desk.name}
        </span>
      </div>

      {/* Role */}
      <div className={`text-[11px] uppercase tracking-wider ${PRESENCE_TEXT[desk.presence]}`}>
        {desk.role}
      </div>

      {/* Spinner + live streaming text (replaces fake progress bar) */}
      {isComputing && (
        <div className="mt-1 flex items-start gap-1 min-h-[2rem]">
          <span
            className="text-blue-500 shrink-0 leading-tight"
            style={{ fontSize: '11px' }}
            aria-hidden="true"
          >
            {spinnerFrame}
          </span>
          {displayText ? (
            <span
              className="text-[11px] leading-tight text-blue-600 overflow-hidden"
              style={{
                display: '-webkit-box',
                WebkitLineClamp: 2,
                WebkitBoxOrient: 'vertical',
                wordBreak: 'break-all',
                opacity: 0.85,
              }}
            >
              {displayText}
            </span>
          ) : (
            <span className="text-[11px] text-gray-400 italic">{taskLabel(desk)}</span>
          )}
        </div>
      )}

      {/* Current task (shown when not computing) */}
      {!isComputing && (
        <div className="mt-1 text-[11px] text-gray-500 truncate">
          {taskLabel(desk)}
        </div>
      )}
    </div>
  );
}
