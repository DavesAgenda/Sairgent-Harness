import { useState } from 'react';
import { motion } from 'motion/react';
import { X, Copy, Check } from 'lucide-react';

interface ArtifactViewerProps {
  title: string;
  agentName: string;
  content: string;
  onClose: () => void;
}

export function ArtifactViewer({ title, agentName, content, onClose }: ArtifactViewerProps) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Fallback: select text approach
    }
  }

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 90,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: '24px',
        backgroundColor: 'rgb(0 0 0 / 0.82)',
      }}
      onClick={onClose}
    >
      {/* Panel */}
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: 12 }}
        transition={{ duration: 0.2, ease: 'easeOut' }}
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(860px, 100%)',
          maxHeight: 'calc(100vh - 48px)',
          backgroundColor: 'var(--ws-bg)',
          border: '1px solid var(--ws-border)',
          borderRadius: 'var(--ws-radius-md)',
          boxShadow: 'var(--ws-shadow-overlay)',
          display: 'flex',
          flexDirection: 'column',
          fontFamily: 'monospace',
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: 'var(--ws-space-md) var(--ws-space-lg)',
            borderBottom: '1px solid var(--ws-border)',
            backgroundColor: 'var(--ws-bg-elevated)',
            flexShrink: 0,
          }}
        >
          <div style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}>
            <div
              style={{
                fontSize: 'var(--ws-font-base)',
                fontWeight: 700,
                color: 'var(--ws-fg-primary)',
                letterSpacing: '0.1em',
                textTransform: 'uppercase',
              }}
            >
              {title}
            </div>
            <div
              style={{
                fontSize: 'var(--ws-font-xs)',
                color: 'var(--ws-fg-muted)',
                letterSpacing: '0.08em',
              }}
            >
              ▸ via {agentName.toUpperCase()}
            </div>
          </div>

          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            {/* Copy button */}
            <button
              onClick={handleCopy}
              style={{
                fontFamily: 'monospace',
                fontSize: '0.65rem',
                letterSpacing: '0.08em',
                color: copied ? 'rgb(9 9 11)' : 'rgb(74 222 128)',
                backgroundColor: copied ? 'rgb(34 197 94)' : 'transparent',
                border: '1px solid rgb(34 197 94 / 0.5)',
                padding: '4px 10px',
                cursor: 'pointer',
                textTransform: 'uppercase',
                display: 'flex',
                alignItems: 'center',
                gap: '5px',
                transition: 'all 0.15s ease',
              }}
              onMouseEnter={(e) => {
                if (copied) return;
                e.currentTarget.style.backgroundColor = 'rgb(34 197 94 / 0.1)';
                e.currentTarget.style.borderColor = 'rgb(74 222 128)';
              }}
              onMouseLeave={(e) => {
                if (copied) return;
                e.currentTarget.style.backgroundColor = 'transparent';
                e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.5)';
              }}
            >
              {copied ? <Check size={11} /> : <Copy size={11} />}
              {copied ? 'COPIED' : 'COPY'}
            </button>

            {/* Close button */}
            <button
              onClick={onClose}
              style={{
                background: 'none',
                border: 'none',
                cursor: 'pointer',
                color: 'rgb(34 197 94 / 0.4)',
                padding: '4px',
                display: 'flex',
                alignItems: 'center',
                transition: 'color 0.15s ease',
              }}
              onMouseEnter={(e) => { e.currentTarget.style.color = 'rgb(74 222 128)'; }}
              onMouseLeave={(e) => { e.currentTarget.style.color = 'rgb(34 197 94 / 0.4)'; }}
            >
              <X size={14} />
            </button>
          </div>
        </div>

        {/* Decorative separator */}
        <div
          style={{
            padding: '0 16px',
            backgroundColor: 'rgb(34 197 94 / 0.03)',
            flexShrink: 0,
            borderBottom: '1px solid rgb(34 197 94 / 0.12)',
          }}
        >
          <span style={{ fontSize: '0.55rem', color: 'rgb(34 197 94 / 0.3)', letterSpacing: '0.05em' }}>
            {'═'.repeat(60)}
          </span>
        </div>

        {/* Content area */}
        <div
          style={{
            flex: 1,
            overflow: 'auto',
            padding: '20px 20px',
          }}
        >
          <pre
            style={{
              fontFamily: 'monospace',
              fontSize: 'var(--ws-font-base)',
              color: 'var(--ws-fg-primary)',
              margin: 0,
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              lineHeight: '1.6',
            }}
          >
            {content}
          </pre>
        </div>

        {/* Footer */}
        <div
          style={{
            padding: '6px 16px',
            borderTop: '1px solid rgb(34 197 94 / 0.15)',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            backgroundColor: 'rgb(34 197 94 / 0.02)',
            flexShrink: 0,
          }}
        >
          <span style={{ fontSize: '0.55rem', color: 'rgb(34 197 94 / 0.25)', letterSpacing: '0.08em' }}>
            {content.length.toLocaleString()} CHARS
          </span>
          <span style={{ fontSize: '0.55rem', color: 'rgb(34 197 94 / 0.25)', letterSpacing: '0.08em' }}>
            ESC / CLICK OUTSIDE TO CLOSE
          </span>
        </div>
      </motion.div>
    </motion.div>
  );
}
