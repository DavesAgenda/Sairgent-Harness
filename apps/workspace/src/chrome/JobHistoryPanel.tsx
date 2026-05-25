import { motion, AnimatePresence } from 'motion/react';
import { X, ClipboardList, RotateCcw, Ban } from 'lucide-react';
import type { JobRecord, SwoStatus } from '../types';

const STATUS_BADGE: Record<SwoStatus, { label: string; color: string; bg: string }> = {
  PENDING:        { label: 'Queued',     color: 'rgb(163 163 163)', bg: 'rgb(163 163 163 / 0.1)' },
  IN_PROGRESS:    { label: 'Running',    color: 'rgb(96 165 250)',  bg: 'rgb(96 165 250 / 0.1)' },
  BLOCKED:        { label: 'Blocked',    color: 'rgb(248 113 113)', bg: 'rgb(248 113 113 / 0.1)' },
  WAITING_REVIEW: { label: 'Review',     color: 'rgb(168 85 247)',  bg: 'rgb(168 85 247 / 0.1)' },
  COMPLETED:      { label: 'Done',       color: 'rgb(74 222 128)',  bg: 'rgb(74 222 128 / 0.1)' },
};

function formatTime(timestamp: number): string {
  const d = new Date(timestamp);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function formatDate(timestamp: number): string {
  const d = new Date(timestamp);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return 'Today';
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (d.toDateString() === yesterday.toDateString()) return 'Yesterday';
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

interface JobHistoryPanelProps {
  jobs: JobRecord[];
  open: boolean;
  onClose: () => void;
  onJobClick: (jobId: string) => void;
  onRerun?: (jobTitle: string) => void;
  onCancel?: (jobId: string) => void;
  viewedJobIds?: Set<string>;
  onMarkAllRead?: () => void;
}

function isNonTerminal(status: SwoStatus): boolean {
  return status === 'PENDING' || status === 'IN_PROGRESS' || status === 'BLOCKED' || status === 'WAITING_REVIEW';
}

export function JobHistoryPanel({ jobs, open, onClose, onJobClick, onRerun, onCancel, viewedJobIds, onMarkAllRead }: JobHistoryPanelProps) {
  const unreadCount = viewedJobIds
    ? jobs.filter((j) => j.status === 'COMPLETED' && !viewedJobIds.has(j.id)).length
    : 0;
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
            className="fixed left-0 top-14 bottom-32 w-80 z-50 flex flex-col"
            style={{ backgroundColor: 'var(--ws-bg)', borderRight: '1px solid var(--ws-border)' }}
            initial={{ x: '-100%' }}
            animate={{ x: 0 }}
            exit={{ x: '-100%' }}
            transition={{ type: 'spring', damping: 25, stiffness: 300 }}
          >
            {/* Header */}
            <div className="flex items-center justify-between px-4 py-3" style={{ borderBottom: '1px solid var(--ws-border-subtle)' }}>
              <div className="flex items-center gap-2" style={{ color: 'var(--ws-fg-primary)' }}>
                <ClipboardList size={16} />
                <span className="font-bold tracking-wider" style={{ fontSize: 'var(--ws-font-base)' }}>JOB HISTORY</span>
              </div>
              <div className="flex items-center gap-2">
                {unreadCount > 0 && onMarkAllRead && (
                  <button
                    onClick={onMarkAllRead}
                    className="text-[10px] text-neutral-500 hover:text-green-400 transition-colors"
                    style={{ fontFamily: 'monospace', letterSpacing: '0.06em' }}
                  >
                    MARK ALL READ
                  </button>
                )}
                <button
                  onClick={onClose}
                  className="text-neutral-500 hover:text-green-400 transition-colors"
                >
                  <X size={16} />
                </button>
              </div>
            </div>

            {/* Job list */}
            <div className="flex-1 overflow-y-auto">
              {jobs.length === 0 && (
                <p className="text-neutral-600 text-xs text-center py-8">
                  No jobs submitted yet.
                </p>
              )}
              {jobs.map((job) => {
                const badge = STATUS_BADGE[job.status];
                const isUnread = job.status === 'COMPLETED' && viewedJobIds && !viewedJobIds.has(job.id);
                return (
                  <div
                    key={job.id}
                    role="button"
                    tabIndex={0}
                    onClick={() => onJobClick(job.id)}
                    onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && onJobClick(job.id)}
                    className="px-4 py-3 cursor-pointer transition-colors hover:bg-neutral-800/50"
                    style={{ borderBottom: '1px solid var(--ws-border-subtle)' }}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex-1 min-w-0">
                        <div
                          className="text-xs font-bold truncate flex items-center gap-2"
                          style={{ color: 'var(--ws-fg-primary)', letterSpacing: '0.05em' }}
                        >
                          {isUnread && (
                            <span
                              style={{
                                width: '6px',
                                height: '6px',
                                borderRadius: '50%',
                                backgroundColor: 'rgb(74 222 128)',
                                flexShrink: 0,
                              }}
                            />
                          )}
                          {job.title}
                        </div>
                        <div className="flex items-center gap-2 mt-1">
                          <span
                            className="text-[10px] px-1.5 py-0.5 rounded-sm font-mono"
                            style={{
                              color: badge.color,
                              backgroundColor: badge.bg,
                              border: `1px solid ${badge.color.replace(')', ' / 0.3)')}`,
                              letterSpacing: '0.06em',
                            }}
                          >
                            {badge.label.toUpperCase()}
                          </span>
                          <span className="text-[10px] text-neutral-500">
                            {job.assigneeName}
                          </span>
                        </div>
                      </div>
                      <div className="flex flex-col items-end gap-1">
                        <span className="text-[10px] text-neutral-600 tabular-nums">
                          {formatDate(job.createdAt)}
                        </span>
                        <span className="text-[10px] text-neutral-600 tabular-nums">
                          {formatTime(job.createdAt)}
                        </span>
                      </div>
                    </div>

                    {/* Deliverable preview for completed jobs */}
                    {job.status === 'COMPLETED' && job.reviewResponse && (
                      <div
                        className="mt-1.5 text-[10px] leading-snug"
                        style={{
                          color: 'var(--ws-fg-muted)',
                          overflow: 'hidden',
                          display: '-webkit-box',
                          WebkitLineClamp: 2,
                          WebkitBoxOrient: 'vertical',
                          opacity: 0.7,
                        }}
                      >
                        {job.reviewResponse.length > 120
                          ? `${job.reviewResponse.slice(0, 117)}...`
                          : job.reviewResponse}
                      </div>
                    )}

                    {/* Re-run button for completed jobs */}
                    {job.status === 'COMPLETED' && onRerun && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          onRerun(job.title);
                        }}
                        className="mt-2 flex items-center gap-1 text-[10px] text-neutral-500 hover:text-green-400 transition-colors"
                        style={{ fontFamily: 'monospace', letterSpacing: '0.06em' }}
                      >
                        <RotateCcw size={10} />
                        RE-RUN
                      </button>
                    )}

                    {/* Cancel button for non-terminal jobs */}
                    {isNonTerminal(job.status) && onCancel && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          onCancel(job.id);
                        }}
                        className="mt-2 flex items-center gap-1 text-[10px] text-neutral-500 hover:text-red-400 transition-colors"
                        style={{ fontFamily: 'monospace', letterSpacing: '0.06em' }}
                      >
                        <Ban size={10} />
                        CANCEL
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
