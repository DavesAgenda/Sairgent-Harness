import { useState, useEffect } from 'react';
import { isTauriRuntime } from '../../sim/platform';
import type { RecurringTemplateView } from '../../types';

// ---------------------------------------------------------------------------
// Schedule parsing helpers
// ---------------------------------------------------------------------------

interface ParsedSchedule {
  type: string;
  cronExpr?: string;
  intervalMinutes?: number;
  days?: string[];
  hour?: number;
  minute?: number;
}

function parseCadence(scheduleJson: string): string {
  try {
    const s: ParsedSchedule = JSON.parse(scheduleJson);
    if (s.type === 'cron' && s.cronExpr) {
      // Try to make cron human-readable
      return `Cron: ${s.cronExpr}`;
    }
    if (s.type === 'daily') {
      const h = s.hour ?? 0;
      const m = s.minute ?? 0;
      const hh = String(h).padStart(2, '0');
      const mm = String(m).padStart(2, '0');
      const days = s.days && s.days.length > 0 ? ` ${s.days.join('/')}` : '';
      return `Daily @ ${hh}:${mm} UTC${days}`;
    }
    if (s.type === 'interval' && s.intervalMinutes) {
      if (s.intervalMinutes < 60) return `Every ${s.intervalMinutes}m`;
      const h = Math.floor(s.intervalMinutes / 60);
      const rem = s.intervalMinutes % 60;
      return rem === 0 ? `Every ${h}h` : `Every ${h}h ${rem}m`;
    }
    if (s.type === 'weekly') {
      const h = s.hour ?? 0;
      const m = s.minute ?? 0;
      const hh = String(h).padStart(2, '0');
      const mm = String(m).padStart(2, '0');
      const days = s.days && s.days.length > 0 ? s.days.join('/') : '?';
      return `Weekly ${days} @ ${hh}:${mm} UTC`;
    }
    return s.type ?? 'Custom';
  } catch {
    return 'Custom';
  }
}

function fmtDateTime(iso: string | null): string {
  if (!iso) return '—';
  try {
    const d = new Date(iso);
    return d.toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });
  } catch {
    return iso;
  }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function SchedulesSection() {
  const [templates, setTemplates] = useState<RecurringTemplateView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [triggeringId, setTriggeringId] = useState<string | null>(null);
  const [triggeredIds, setTriggeredIds] = useState<Set<string>>(new Set());

  useEffect(() => {
    loadTemplates();
  }, []);

  async function loadTemplates() {
    setLoading(true);
    setError(null);
    if (isTauriRuntime()) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const result = await invoke<RecurringTemplateView[]>('recurring_templates_list');
        setTemplates(result);
      } catch (err) {
        setError(`Failed to load schedules: ${err}`);
      }
    } else {
      // Mock mode: empty list
      setTemplates([]);
    }
    setLoading(false);
  }

  async function handleTrigger(template: RecurringTemplateView) {
    setTriggeringId(template.templateId);
    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('recurring_template_trigger', { templateId: template.templateId });
      }
      setTriggeredIds((prev) => new Set(prev).add(template.templateId));
      // Clear the "triggered" indicator after 3 seconds
      setTimeout(() => {
        setTriggeredIds((prev) => {
          const next = new Set(prev);
          next.delete(template.templateId);
          return next;
        });
      }, 3000);
    } catch (err) {
      setError(`Trigger failed: ${err}`);
    } finally {
      setTriggeringId(null);
    }
  }

  return (
    <div>
      <h2 style={headingStyle}>Schedules</h2>
      <p style={subTextStyle}>
        Recurring work order templates. Each schedule fires a new job at the configured cadence.
        Use "Trigger Now" to run a schedule immediately outside its normal window.
      </p>

      {/* Error banner */}
      {error && (
        <div style={errorBannerStyle}>
          {error}
          <button style={clearBtnStyle} onClick={() => setError(null)}>
            dismiss
          </button>
        </div>
      )}

      {/* Reload toolbar */}
      <div style={{ marginBottom: '16px' }}>
        <button style={btnStyle} onClick={loadTemplates} disabled={loading}>
          {loading ? 'Loading...' : '↻ Refresh'}
        </button>
      </div>

      {/* Table */}
      {loading ? (
        <div style={emptyStateStyle}>Loading schedules...</div>
      ) : templates.length === 0 ? (
        <div style={emptyStateStyle}>
          No recurring schedules configured. Create a recurring template via the kernel seed or API.
        </div>
      ) : (
        <table style={tableStyle}>
          <thead>
            <tr>
              {(['Name', 'Cadence', 'Status', 'Last Run', 'Next Run', ''] as const).map((h) => (
                <th key={h} style={thStyle}>
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {templates.map((t) => (
              <tr key={t.templateId} style={trStyle}>
                {/* Name + title */}
                <td style={tdStyle}>
                  <span style={{ color: 'rgb(74 222 128)', fontWeight: 600 }}>{t.name}</span>
                  {t.title && t.title !== t.name && (
                    <div style={{ color: 'rgb(34 197 94 / 0.5)', fontSize: '0.6rem', marginTop: '2px' }}>
                      {t.title}
                    </div>
                  )}
                  {t.assigneeAgentName && (
                    <div style={{ color: 'rgb(34 197 94 / 0.4)', fontSize: '0.58rem', marginTop: '2px' }}>
                      → {t.assigneeAgentName}
                    </div>
                  )}
                </td>

                {/* Cadence */}
                <td style={tdStyle}>
                  <code style={codeStyle}>{parseCadence(t.scheduleJson)}</code>
                </td>

                {/* Status badge */}
                <td style={tdStyle}>
                  <span style={statusBadgeStyle(t.status)}>
                    {t.status}
                  </span>
                </td>

                {/* Last run */}
                <td style={tdStyle}>
                  <span style={{ color: 'rgb(74 222 128 / 0.7)', fontSize: '0.65rem' }}>
                    {fmtDateTime(t.lastRunAt)}
                  </span>
                  {t.lastRunStatus && (
                    <div style={{ marginTop: '2px' }}>
                      <span style={runStatusBadgeStyle(t.lastRunStatus)}>
                        {t.lastRunStatus}
                      </span>
                    </div>
                  )}
                </td>

                {/* Next run */}
                <td style={tdStyle}>
                  <span style={{ color: 'rgb(74 222 128 / 0.7)', fontSize: '0.65rem' }}>
                    {fmtDateTime(t.nextRunAt)}
                  </span>
                </td>

                {/* Actions */}
                <td style={{ ...tdStyle, textAlign: 'right' }}>
                  <button
                    style={
                      triggeredIds.has(t.templateId)
                        ? { ...actionBtnStyle, color: 'rgb(74 222 128)', borderColor: 'rgb(34 197 94 / 0.6)' }
                        : actionBtnStyle
                    }
                    onClick={() => handleTrigger(t)}
                    disabled={triggeringId === t.templateId}
                  >
                    {triggeringId === t.templateId
                      ? '...'
                      : triggeredIds.has(t.templateId)
                        ? '✓ Triggered'
                        : 'Trigger Now'}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const headingStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.85rem',
  fontWeight: 700,
  color: 'rgb(74 222 128)',
  letterSpacing: '0.1em',
  textTransform: 'uppercase',
  marginBottom: '8px',
};

const subTextStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(34 197 94 / 0.6)',
  marginBottom: '24px',
  lineHeight: 1.5,
};

const btnStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  letterSpacing: '0.1em',
  textTransform: 'uppercase',
  color: 'rgb(74 222 128)',
  backgroundColor: 'transparent',
  border: '1px solid rgb(34 197 94 / 0.5)',
  padding: '8px 20px',
  cursor: 'pointer',
};

const actionBtnStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.6rem',
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  color: 'rgb(74 222 128)',
  backgroundColor: 'transparent',
  border: '1px solid rgb(34 197 94 / 0.3)',
  padding: '4px 10px',
  cursor: 'pointer',
};

const errorBannerStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(239 68 68 / 0.8)',
  border: '1px solid rgb(239 68 68 / 0.3)',
  borderRadius: '3px',
  padding: '8px 12px',
  marginBottom: '16px',
  display: 'flex',
  justifyContent: 'space-between',
  alignItems: 'center',
};

const clearBtnStyle: React.CSSProperties = {
  background: 'none',
  border: 'none',
  cursor: 'pointer',
  fontFamily: 'monospace',
  fontSize: '0.65rem',
  color: 'rgb(239 68 68 / 0.5)',
  textTransform: 'uppercase',
};

const tableStyle: React.CSSProperties = {
  width: '100%',
  borderCollapse: 'collapse',
  fontFamily: 'monospace',
  fontSize: '0.7rem',
};

const thStyle: React.CSSProperties = {
  textAlign: 'left',
  fontFamily: 'monospace',
  fontSize: '0.62rem',
  fontWeight: 700,
  color: 'rgb(34 197 94 / 0.5)',
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  padding: '6px 10px',
  borderBottom: '1px solid rgb(34 197 94 / 0.15)',
};

const tdStyle: React.CSSProperties = {
  padding: '10px 10px',
  borderBottom: '1px solid rgb(34 197 94 / 0.08)',
  verticalAlign: 'middle',
  color: 'rgb(74 222 128)',
};

const trStyle: React.CSSProperties = {
  transition: 'background-color 0.1s ease',
};

const codeStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.65rem',
  color: 'rgb(74 222 128 / 0.8)',
};

const emptyStateStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(34 197 94 / 0.4)',
  padding: '20px 0',
};

function statusBadgeStyle(status: string): React.CSSProperties {
  const active = status === 'ACTIVE';
  return {
    fontFamily: 'monospace',
    fontSize: '0.58rem',
    letterSpacing: '0.07em',
    textTransform: 'uppercase',
    color: active ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.35)',
    border: `1px solid ${active ? 'rgb(34 197 94 / 0.5)' : 'rgb(34 197 94 / 0.15)'}`,
    borderRadius: '3px',
    padding: '2px 6px',
  };
}

function runStatusBadgeStyle(status: string): React.CSSProperties {
  const ok = status === 'COMPLETED' || status === 'SUCCESS';
  const failed = status === 'FAILED' || status === 'ERROR';
  let color = 'rgb(74 222 128 / 0.6)';
  let border = 'rgb(34 197 94 / 0.2)';
  if (ok) { color = 'rgb(74 222 128)'; border = 'rgb(34 197 94 / 0.5)'; }
  if (failed) { color = 'rgb(239 68 68 / 0.8)'; border = 'rgb(239 68 68 / 0.3)'; }
  return {
    fontFamily: 'monospace',
    fontSize: '0.55rem',
    letterSpacing: '0.06em',
    textTransform: 'uppercase',
    color,
    border: `1px solid ${border}`,
    borderRadius: '3px',
    padding: '1px 5px',
  };
}
