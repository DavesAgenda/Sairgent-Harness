import { motion, AnimatePresence } from 'motion/react';
import { Inbox } from 'lucide-react';
import type { InboxItem } from '../../../types';

interface InboxTrayProps {
  items: InboxItem[];
  onItemClick: (itemId: string) => void;
}

function relativeTime(timestamp: number): string {
  const seconds = Math.floor((Date.now() - timestamp) / 1000);
  if (seconds < 5) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export function InboxTray({ items, onItemClick }: InboxTrayProps) {
  return (
    <div
      data-inbox-tray
      style={{
        position: 'fixed',
        bottom: '48px',  // sits above DevToolbar
        left: 0,
        right: 0,
        zIndex: 55,
        backgroundColor: 'var(--ws-bg)',
        borderTop: '1px solid var(--ws-border-subtle)',
        fontFamily: 'monospace',
      }}
    >
      {/* Tray header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          padding: '5px 14px 4px',
          borderBottom: '1px solid rgb(34 197 94 / 0.12)',
        }}
      >
        <Inbox size={13} style={{ color: 'var(--ws-fg-muted)' }} />
        <span
          style={{
            fontSize: 'var(--ws-font-xs)',
            color: 'var(--ws-fg-muted)',
            letterSpacing: '0.15em',
            textTransform: 'uppercase',
          }}
        >
          DELIVERABLES
        </span>
        {items.length > 0 && (
          <span
            style={{
              fontSize: 'var(--ws-font-xs)',
              color: 'var(--ws-fg-dim)',
              letterSpacing: '0.08em',
            }}
          >
            [{items.length}]
          </span>
        )}
      </div>

      {/* Scrollable card row */}
      <div
        style={{
          overflowX: 'auto',
          overflowY: 'hidden',
          padding: '8px 12px',
          display: 'flex',
          gap: '8px',
          minHeight: '68px',
          alignItems: 'center',
          scrollbarWidth: 'thin',
          scrollbarColor: 'rgb(34 197 94 / 0.2) transparent',
        }}
      >
        <AnimatePresence initial={false}>
          {items.length === 0 ? (
            <motion.div
              key="empty"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              style={{
                fontSize: 'var(--ws-font-sm)',
                color: 'var(--ws-fg-dim)',
                letterSpacing: '0.08em',
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                padding: '0 4px',
                whiteSpace: 'nowrap',
              }}
            >
              <span style={{ opacity: 0.5 }}>{'\u2504'}</span>
              No deliverables yet
              <span style={{ opacity: 0.5 }}>{'\u2504'}</span>
            </motion.div>
          ) : (
            items.map((item) => (
              <InboxCard
                key={item.id}
                item={item}
                onClick={() => onItemClick(item.id)}
              />
            ))
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}

interface InboxCardProps {
  item: InboxItem;
  onClick: () => void;
}

function InboxCard({ item, onClick }: InboxCardProps) {
  return (
    <motion.button
      key={item.id}
      initial={{ opacity: 0, x: 60 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ type: 'spring', stiffness: 300, damping: 28 }}
      onClick={onClick}
      style={{
        fontFamily: 'monospace',
        flexShrink: 0,
        width: '210px',
        backgroundColor: 'var(--ws-bg-elevated)',
        border: '1px solid var(--ws-border)',
        borderRadius: 'var(--ws-radius-sm)',
        padding: 'var(--ws-space-sm) var(--ws-space-md)',
        cursor: 'pointer',
        textAlign: 'left',
        transition: 'all 0.15s ease',
        display: 'flex',
        flexDirection: 'column',
        gap: '6px',
      }}
      onMouseEnter={(e) => {
        const el = e.currentTarget;
        el.style.backgroundColor = 'var(--ws-accent-soft)';
        el.style.borderColor = 'var(--ws-border-bright)';
        el.style.boxShadow = 'var(--ws-shadow-glow)';
      }}
      onMouseLeave={(e) => {
        const el = e.currentTarget;
        el.style.backgroundColor = 'var(--ws-bg-elevated)';
        el.style.borderColor = 'var(--ws-border)';
        el.style.boxShadow = 'none';
      }}
    >
      {/* Top row: agent icon + name */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
          <div
            style={{
              width: '18px',
              height: '18px',
              border: '1px solid rgb(34 197 94 / 0.4)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              fontSize: '0.65rem',
              color: 'rgb(74 222 128)',
              backgroundColor: 'rgb(34 197 94 / 0.06)',
              flexShrink: 0,
            }}
          >
            {'\u25C8'}
          </div>
          <span
            style={{
              fontSize: 'var(--ws-font-xs)',
              color: 'var(--ws-fg-primary)',
              letterSpacing: '0.06em',
              textTransform: 'uppercase',
              fontWeight: 600,
            }}
          >
            {item.agentName}
          </span>
        </div>
        <span
          style={{
            fontSize: 'var(--ws-font-xs)',
            color: 'var(--ws-fg-dim)',
            letterSpacing: '0.05em',
            whiteSpace: 'nowrap',
          }}
        >
          {relativeTime(item.timestamp)}
        </span>
      </div>

      {/* Title */}
      <div
        style={{
          fontSize: 'var(--ws-font-sm)',
          color: 'var(--ws-fg-primary)',
          letterSpacing: '0.03em',
          lineHeight: '1.3',
          overflow: 'hidden',
          display: '-webkit-box',
          WebkitLineClamp: 2,
          WebkitBoxOrient: 'vertical',
        }}
      >
        {item.title}
      </div>

      {/* CTA hint */}
      <div
        style={{
          fontSize: 'var(--ws-font-xs)',
          color: 'var(--ws-fg-dim)',
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
        }}
      >
        CLICK TO VIEW {'\u203A'}
      </div>
    </motion.button>
  );
}
