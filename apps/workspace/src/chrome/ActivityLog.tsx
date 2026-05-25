import { useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { X, Activity } from 'lucide-react';
import type { ActivityLogEntry } from '../types';

const KIND_ICONS: Record<string, string> = {
  task_started: '▶',
  task_completed: '✓',
  delegated: '↗',
  blocked: '⊘',
  artifact_produced: '◆',
  presence_changed: '◎',
};

const KIND_COLORS: Record<string, string> = {
  task_started: 'text-blue-400',
  task_completed: 'text-green-400',
  delegated: 'text-purple-400',
  blocked: 'text-red-400',
  artifact_produced: 'text-yellow-400',
  presence_changed: 'text-neutral-500',
};

function formatTime(timestamp: number): string {
  const d = new Date(timestamp);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

export function ActivityLog({
  entries,
  open,
  onClose,
  onEntryClick,
}: {
  entries: ActivityLogEntry[];
  open: boolean;
  onClose: () => void;
  onEntryClick?: (swoId: string) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when new entries arrive
  useEffect(() => {
    if (open && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [entries.length, open]);

  return (
    <AnimatePresence>
      {open && (
        <>
          {/* Backdrop */}
          <motion.div
            className="fixed inset-0 bg-black/30 z-40"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={onClose}
          />

          {/* Panel */}
          <motion.div
            className="fixed right-0 top-14 bottom-32 w-80 z-50 flex flex-col"
            style={{ backgroundColor: 'var(--ws-bg)', borderLeft: '1px solid var(--ws-border)' }}
            initial={{ x: '100%' }}
            animate={{ x: 0 }}
            exit={{ x: '100%' }}
            transition={{ type: 'spring', damping: 25, stiffness: 300 }}
          >
            {/* Header */}
            <div className="flex items-center justify-between px-4 py-3" style={{ borderBottom: '1px solid var(--ws-border-subtle)' }}>
              <div className="flex items-center gap-2" style={{ color: 'var(--ws-fg-primary)' }}>
                <Activity size={16} />
                <span className="font-bold tracking-wider" style={{ fontSize: 'var(--ws-font-base)' }}>ACTIVITY LOG</span>
              </div>
              <button
                onClick={onClose}
                className="text-neutral-500 hover:text-green-400 transition-colors"
              >
                <X size={16} />
              </button>
            </div>

            {/* Entries */}
            <div ref={scrollRef} className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
              {entries.length === 0 && (
                <p className="text-neutral-600 text-xs text-center py-8">
                  No activity yet. Submit a job to see the flow.
                </p>
              )}
              {entries.map((entry) => (
                <div
                  key={entry.id}
                  role={entry.swoId && onEntryClick ? 'button' : undefined}
                  tabIndex={entry.swoId && onEntryClick ? 0 : undefined}
                  onClick={() => entry.swoId && onEntryClick?.(entry.swoId)}
                  onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && entry.swoId && onEntryClick?.(entry.swoId)}
                  className={`flex items-start gap-2 text-xs py-1 border-b border-neutral-800/50 ${entry.swoId && onEntryClick ? 'cursor-pointer hover:bg-neutral-800/30' : ''}`}
                >
                  <span className={`${KIND_COLORS[entry.kind] ?? 'text-neutral-500'} flex-shrink-0 w-3`}>
                    {KIND_ICONS[entry.kind] ?? '·'}
                  </span>
                  <div className="flex-1 min-w-0">
                    <span className="text-green-300 font-medium">{entry.agentName}</span>
                    <span className="text-neutral-500 ml-1">{entry.summary}</span>
                  </div>
                  <span className="text-neutral-600 flex-shrink-0 tabular-nums">
                    {formatTime(entry.timestamp)}
                  </span>
                </div>
              ))}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
