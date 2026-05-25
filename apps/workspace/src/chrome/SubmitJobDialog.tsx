import { useState } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { X, Send } from 'lucide-react';

interface SubmitJobDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (title: string, description: string) => void;
}

export function SubmitJobDialog({ open, onOpenChange, onSubmit }: SubmitJobDialogProps) {
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');

  function handleSubmit() {
    if (!title.trim()) return;
    onSubmit(title.trim(), description.trim());
    setTitle('');
    setDescription('');
    onOpenChange(false);
  }

  function handleCancel() {
    setTitle('');
    setDescription('');
    onOpenChange(false);
  }

  const inputStyle: React.CSSProperties = {
    fontFamily: 'monospace',
    fontSize: 'var(--ws-font-md)',
    color: 'var(--ws-fg-primary)',
    backgroundColor: 'var(--ws-bg)',
    border: '1px solid var(--ws-border)',
    borderRadius: 'var(--ws-radius-sm)',
    padding: 'var(--ws-space-sm) var(--ws-space-md)',
    width: '100%',
    outline: 'none',
    resize: 'none' as const,
    transition: 'border-color 0.15s ease',
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        {/* Overlay */}
        <Dialog.Overlay
          style={{
            position: 'fixed',
            inset: 0,
            backgroundColor: 'rgb(0 0 0 / 0.75)',
            zIndex: 100,
          }}
        />

        {/* Content */}
        <Dialog.Content
          style={{
            position: 'fixed',
            top: '50%',
            left: '50%',
            transform: 'translate(-50%, -50%)',
            zIndex: 101,
            width: 'min(520px, calc(100vw - 48px))',
            backgroundColor: 'var(--ws-bg)',
            border: '1px solid var(--ws-border)',
            borderRadius: 'var(--ws-radius-md)',
            boxShadow: 'var(--ws-shadow-overlay)',
            fontFamily: 'monospace',
            outline: 'none',
          }}
        >
          {/* Header bar */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              padding: 'var(--ws-space-md) var(--ws-space-lg)',
              borderBottom: '1px solid var(--ws-border)',
              backgroundColor: 'var(--ws-accent-soft)',
            }}
          >
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--ws-space-sm)' }}>
              <span style={{ color: 'var(--ws-fg-muted)', fontSize: 'var(--ws-font-sm)' }}>┌─</span>
              <Dialog.Title
                style={{
                  fontFamily: 'monospace',
                  fontSize: 'var(--ws-font-base)',
                  fontWeight: 700,
                  color: 'var(--ws-fg-primary)',
                  letterSpacing: '0.12em',
                  textTransform: 'uppercase',
                  margin: 0,
                }}
              >
                SUBMIT JOB
              </Dialog.Title>
              <span style={{ color: 'var(--ws-fg-muted)', fontSize: 'var(--ws-font-sm)' }}>─┐</span>
            </div>

            <Dialog.Close asChild>
              <button
                onClick={handleCancel}
                style={{
                  background: 'none',
                  border: 'none',
                  cursor: 'pointer',
                  color: 'rgb(34 197 94 / 0.5)',
                  padding: '2px',
                  display: 'flex',
                  alignItems: 'center',
                  transition: 'color 0.15s ease',
                }}
                onMouseEnter={(e) => { e.currentTarget.style.color = 'rgb(74 222 128)'; }}
                onMouseLeave={(e) => { e.currentTarget.style.color = 'rgb(34 197 94 / 0.5)'; }}
              >
                <X size={14} />
              </button>
            </Dialog.Close>
          </div>

          {/* Body */}
          <div style={{ padding: '20px 18px', display: 'flex', flexDirection: 'column', gap: '16px' }}>
            {/* Job Title field */}
            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
              <label
                style={{
                  fontFamily: 'monospace',
                  fontSize: 'var(--ws-font-sm)',
                  color: 'var(--ws-fg-secondary)',
                  letterSpacing: '0.1em',
                  textTransform: 'uppercase',
                }}
              >
                JOB TITLE
              </label>
              <input
                type="text"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="e.g. Market analysis report..."
                style={{
                  ...inputStyle,
                }}
                onFocus={(e) => { e.currentTarget.style.borderColor = 'rgb(74 222 128)'; e.currentTarget.style.boxShadow = '0 0 6px rgb(34 197 94 / 0.2)'; }}
                onBlur={(e) => { e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.4)'; e.currentTarget.style.boxShadow = 'none'; }}
                onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) handleSubmit(); }}
                autoFocus
              />
            </div>

            {/* Description field */}
            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
              <label
                style={{
                  fontFamily: 'monospace',
                  fontSize: 'var(--ws-font-sm)',
                  color: 'var(--ws-fg-secondary)',
                  letterSpacing: '0.1em',
                  textTransform: 'uppercase',
                }}
              >
                DESCRIPTION
              </label>
              <textarea
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Optional: additional context, requirements, constraints..."
                rows={5}
                style={{
                  ...inputStyle,
                  resize: 'vertical',
                }}
                onFocus={(e) => { e.currentTarget.style.borderColor = 'rgb(74 222 128)'; e.currentTarget.style.boxShadow = '0 0 6px rgb(34 197 94 / 0.2)'; }}
                onBlur={(e) => { e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.4)'; e.currentTarget.style.boxShadow = 'none'; }}
              />
            </div>
          </div>

          {/* Footer */}
          <div
            style={{
              display: 'flex',
              justifyContent: 'flex-end',
              gap: '10px',
              padding: '10px 18px 16px',
            }}
          >
            {/* Cancel */}
            <button
              onClick={handleCancel}
              style={{
                fontFamily: 'monospace',
                fontSize: 'var(--ws-font-sm)',
                letterSpacing: '0.08em',
                color: 'var(--ws-fg-muted)',
                backgroundColor: 'transparent',
                border: '1px solid var(--ws-border-subtle)',
                borderRadius: 'var(--ws-radius-sm)',
                padding: '6px 14px',
                cursor: 'pointer',
                textTransform: 'uppercase',
                transition: 'all 0.15s ease',
              }}
              onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--ws-fg-primary)'; e.currentTarget.style.borderColor = 'var(--ws-border)'; }}
              onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--ws-fg-muted)'; e.currentTarget.style.borderColor = 'var(--ws-border-subtle)'; }}
            >
              CANCEL
            </button>

            {/* Submit */}
            <button
              onClick={handleSubmit}
              disabled={!title.trim()}
              style={{
                fontFamily: 'monospace',
                fontSize: 'var(--ws-font-sm)',
                letterSpacing: '0.08em',
                color: title.trim() ? 'var(--ws-bg)' : 'var(--ws-fg-dim)',
                backgroundColor: title.trim() ? 'var(--ws-accent)' : 'transparent',
                border: `1px solid ${title.trim() ? 'var(--ws-accent)' : 'var(--ws-border-subtle)'}`,
                borderRadius: 'var(--ws-radius-sm)',
                padding: '6px 16px',
                cursor: title.trim() ? 'pointer' : 'not-allowed',
                textTransform: 'uppercase',
                display: 'flex',
                alignItems: 'center',
                gap: '6px',
                transition: 'all 0.15s ease',
              }}
              onMouseEnter={(e) => {
                if (!title.trim()) return;
                e.currentTarget.style.backgroundColor = 'var(--ws-fg-primary)';
                e.currentTarget.style.boxShadow = 'var(--ws-shadow-glow)';
              }}
              onMouseLeave={(e) => {
                if (!title.trim()) return;
                e.currentTarget.style.backgroundColor = 'var(--ws-accent)';
                e.currentTarget.style.boxShadow = 'none';
              }}
            >
              <Send size={12} />
              DISPATCH
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
