import type { InboxItem } from '../../../types';
import { emojiIcon } from './agentIcons';

interface Props {
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

export function EmojiInboxTray({ items, onItemClick }: Props) {
  return (
    <div
      style={{
        position: 'fixed',
        bottom: '48px',
        left: 0,
        right: 0,
        zIndex: 55,
        backgroundColor: 'rgba(255, 255, 255, 0.97)',
        borderTop: '2px solid #e5e7eb',
      }}
    >
      {/* Tray header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          padding: '6px 14px 4px',
          borderBottom: '1px solid #f3f4f6',
        }}
      >
        <span style={{ fontSize: '14px' }}>{'\uD83D\uDCE5'}</span>
        <span
          style={{
            fontSize: '0.65rem',
            color: '#9ca3af',
            letterSpacing: '0.12em',
            textTransform: 'uppercase',
            fontWeight: 600,
          }}
        >
          DELIVERABLES
        </span>
        {items.length > 0 && (
          <span
            style={{
              fontSize: '0.6rem',
              color: '#6b7280',
              fontWeight: 600,
            }}
          >
            ({items.length})
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
          gap: '10px',
          minHeight: '68px',
          alignItems: 'center',
        }}
      >
        {items.length === 0 ? (
          <div
            style={{
              fontSize: '0.75rem',
              color: '#d1d5db',
              letterSpacing: '0.06em',
              padding: '0 4px',
            }}
          >
            No deliverables yet
          </div>
        ) : (
          items.map((item) => (
            <button
              key={item.id}
              onClick={() => onItemClick(item.id)}
              style={{
                flexShrink: 0,
                width: '200px',
                backgroundColor: '#f9fafb',
                border: '1px solid #e5e7eb',
                borderRadius: '10px',
                padding: '8px 10px',
                cursor: 'pointer',
                textAlign: 'left',
                transition: 'all 0.15s ease',
                display: 'flex',
                flexDirection: 'column',
                gap: '4px',
              }}
            >
              {/* Agent row */}
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                  <span style={{ fontSize: '16px' }}>{emojiIcon(item.agentName)}</span>
                  <span
                    style={{
                      fontSize: '0.65rem',
                      color: '#374151',
                      fontWeight: 600,
                      textTransform: 'uppercase',
                      letterSpacing: '0.04em',
                    }}
                  >
                    {item.agentName}
                  </span>
                </div>
                <span style={{ fontSize: '0.55rem', color: '#9ca3af' }}>
                  {relativeTime(item.timestamp)}
                </span>
              </div>

              {/* Title */}
              <div
                style={{
                  fontSize: '0.72rem',
                  color: '#111827',
                  lineHeight: '1.3',
                  overflow: 'hidden',
                  display: '-webkit-box',
                  WebkitLineClamp: 2,
                  WebkitBoxOrient: 'vertical',
                }}
              >
                {item.title}
              </div>
            </button>
          ))
        )}
      </div>
    </div>
  );
}
