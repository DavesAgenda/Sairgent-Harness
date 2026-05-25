import { Activity, ClipboardList, Inbox, Settings } from 'lucide-react';

interface HeaderProps {
  inboxCount: number;
  activityCount?: number;
  jobCount?: number;
  onSubmitClick: () => void;
  onInboxClick: () => void;
  onActivityClick?: () => void;
  onJobHistoryClick?: () => void;
  onSettingsClick?: () => void;
  /** Optional slot rendered between the activity button and the inbox button. */
  extraControls?: React.ReactNode;
}

export function Header({ inboxCount, activityCount, jobCount, onSubmitClick, onInboxClick, onActivityClick, onJobHistoryClick, onSettingsClick, extraControls }: HeaderProps) {
  return (
    <header
      className="fixed top-0 left-0 right-0 z-50 flex items-center justify-between px-6 py-3"
      style={{
        backgroundColor: 'var(--ws-bg)',
        borderBottom: '1px solid var(--ws-border)',
        fontFamily: 'monospace',
      }}
    >
      {/* Left: App title */}
      <div className="flex items-center gap-3">
        <span
          style={{
            fontSize: 'var(--ws-font-sm)',
            color: 'var(--ws-fg-muted)',
            letterSpacing: '0.05em',
          }}
        >
          ┌─
        </span>
        <h1
          style={{
            fontFamily: 'monospace',
            fontSize: 'var(--ws-font-xl)',
            fontWeight: 700,
            color: 'var(--ws-fg-primary)',
            letterSpacing: '0.15em',
            textTransform: 'uppercase',
          }}
        >
          SAIRGENT WORKSPACE
        </h1>
        <span
          style={{
            fontSize: 'var(--ws-font-sm)',
            color: 'var(--ws-fg-muted)',
            letterSpacing: '0.05em',
          }}
        >
          ─┐
        </span>
      </div>

      {/* Right: Actions */}
      <div className="flex items-center gap-4">
        {/* Job history button */}
        {onJobHistoryClick && (
          <button
            onClick={onJobHistoryClick}
            className="relative flex items-center gap-2"
            style={{
              background: 'none',
              border: '1px solid transparent',
              padding: '4px 8px',
              cursor: 'pointer',
              transition: 'all 0.15s ease',
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.4)';
              e.currentTarget.style.backgroundColor = 'rgb(34 197 94 / 0.06)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.borderColor = 'transparent';
              e.currentTarget.style.backgroundColor = 'transparent';
            }}
          >
            <ClipboardList
              size={16}
              style={{ color: (jobCount ?? 0) > 0 ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.4)' }}
            />
          </button>
        )}

        {/* Activity log button */}
        {onActivityClick && (
          <button
            onClick={onActivityClick}
            className="relative flex items-center gap-2"
            style={{
              background: 'none',
              border: '1px solid transparent',
              padding: '4px 8px',
              cursor: 'pointer',
              transition: 'all 0.15s ease',
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.4)';
              e.currentTarget.style.backgroundColor = 'rgb(34 197 94 / 0.06)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.borderColor = 'transparent';
              e.currentTarget.style.backgroundColor = 'transparent';
            }}
          >
            <Activity
              size={16}
              style={{ color: (activityCount ?? 0) > 0 ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.4)' }}
            />
          </button>
        )}

        {/* Extra controls slot (e.g. skin selector) */}
        {extraControls}

        {/* Inbox button */}
        <button
          onClick={onInboxClick}
          className="relative flex items-center gap-2"
          style={{
            background: 'none',
            border: '1px solid transparent',
            padding: '4px 8px',
            cursor: 'pointer',
            transition: 'all 0.15s ease',
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.4)';
            e.currentTarget.style.backgroundColor = 'rgb(34 197 94 / 0.06)';
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.borderColor = 'transparent';
            e.currentTarget.style.backgroundColor = 'transparent';
          }}
        >
          <Inbox
            size={16}
            style={{ color: inboxCount > 0 ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.4)' }}
          />
          {inboxCount > 0 && (
            <span
              style={{
                position: 'absolute',
                top: '-2px',
                right: '0px',
                minWidth: '16px',
                height: '16px',
                borderRadius: '50%',
                backgroundColor: 'rgb(34 197 94)',
                color: 'rgb(9 9 11)',
                fontSize: '0.6rem',
                fontWeight: 700,
                fontFamily: 'monospace',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                padding: '0 3px',
              }}
            >
              {inboxCount > 99 ? '99+' : inboxCount}
            </span>
          )}
        </button>

        {/* Settings gear button */}
        {onSettingsClick && (
          <button
            onClick={onSettingsClick}
            className="relative flex items-center gap-2"
            style={{
              background: 'none',
              border: '1px solid transparent',
              padding: '4px 8px',
              cursor: 'pointer',
              transition: 'all 0.15s ease',
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.4)';
              e.currentTarget.style.backgroundColor = 'rgb(34 197 94 / 0.06)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.borderColor = 'transparent';
              e.currentTarget.style.backgroundColor = 'transparent';
            }}
            aria-label="Open settings"
          >
            <Settings size={16} style={{ color: 'rgb(34 197 94 / 0.5)' }} />
          </button>
        )}

        {/* Submit Job button */}
        <button
          onClick={onSubmitClick}
          style={{
            fontFamily: 'monospace',
            fontSize: 'var(--ws-font-base)',
            letterSpacing: '0.1em',
            color: 'var(--ws-fg-primary)',
            backgroundColor: 'transparent',
            border: '1px solid var(--ws-border)',
            padding: '6px 16px',
            cursor: 'pointer',
            textTransform: 'uppercase',
            transition: 'all 0.15s ease',
          }}
          onMouseEnter={(e) => {
            const btn = e.currentTarget;
            btn.style.backgroundColor = 'var(--ws-accent-soft)';
            btn.style.borderColor = 'var(--ws-border-bright)';
            btn.style.boxShadow = 'var(--ws-shadow-glow)';
          }}
          onMouseLeave={(e) => {
            const btn = e.currentTarget;
            btn.style.backgroundColor = 'transparent';
            btn.style.borderColor = 'var(--ws-border)';
            btn.style.boxShadow = 'none';
          }}
        >
          [ SUBMIT JOB ]
        </button>
      </div>
    </header>
  );
}
