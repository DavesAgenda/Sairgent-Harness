import { useState, useEffect } from 'react';
import { isTauriRuntime } from '../../sim/platform';
import type { McpConnectorView, McpConnectorUpsertRequest } from '../../types';

const SLUG_RE = /^[a-z0-9_-]+$/;
const MAX_SLUG_LEN = 64;

function deriveSlug(name: string): string {
  return name
    .toLowerCase()
    .trim()
    .replace(/\s+/g, '-')
    .replace(/[^a-z0-9_-]/g, '')
    .slice(0, MAX_SLUG_LEN);
}

interface FormState {
  connectorId?: string;
  name: string;
  slug: string;
  summary: string;
  transport: 'stdio' | 'sse';
  command: string;
  argsText: string;
  url: string;
  enabled: boolean;
}

const emptyForm = (): FormState => ({
  connectorId: undefined,
  name: '',
  slug: '',
  summary: '',
  transport: 'stdio',
  command: '',
  argsText: '',
  url: '',
  enabled: true,
});

function validateForm(form: FormState): string | null {
  if (!form.name.trim()) return 'Name is required';
  if (!form.slug) return 'Slug is required';
  if (form.slug.length > MAX_SLUG_LEN) return `Slug must be ${MAX_SLUG_LEN} characters or fewer`;
  if (!SLUG_RE.test(form.slug)) return 'Slug must only contain lowercase letters, numbers, - or _';
  if (form.transport === 'stdio' && !form.command.trim()) return 'Command is required for stdio transport';
  if (form.transport === 'sse' && !form.url.trim()) return 'URL is required for SSE transport';
  return null;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function McpSection() {
  const [connectors, setConnectors] = useState<McpConnectorView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<FormState>(emptyForm());
  const [slugEdited, setSlugEdited] = useState(false);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [togglingId, setTogglingId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  useEffect(() => {
    loadConnectors();
  }, []);

  async function loadConnectors() {
    setLoading(true);
    setError(null);
    if (isTauriRuntime()) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const result = await invoke<McpConnectorView[]>('mcp_connectors_list');
        setConnectors(result);
      } catch (err) {
        setError(`Failed to load MCP connectors: ${err}`);
      }
    } else {
      // Mock mode: empty list
      setConnectors([]);
    }
    setLoading(false);
  }

  function openAdd() {
    setForm(emptyForm());
    setSlugEdited(false);
    setFormError(null);
    setShowForm(true);
    setConfirmDeleteId(null);
  }

  function openEdit(connector: McpConnectorView) {
    const argsText = connector.args ? connector.args.join(' ') : '';
    setForm({
      connectorId: connector.id,
      name: connector.name,
      slug: connector.slug,
      summary: connector.summary ?? '',
      transport: connector.transport === 'sse' ? 'sse' : 'stdio',
      command: connector.command ?? '',
      argsText,
      url: connector.url ?? '',
      enabled: connector.enabled,
    });
    setSlugEdited(true);
    setFormError(null);
    setShowForm(true);
    setConfirmDeleteId(null);
  }

  function cancelForm() {
    setShowForm(false);
    setFormError(null);
    setForm(emptyForm());
  }

  function handleNameChange(name: string) {
    setForm((prev) => ({
      ...prev,
      name,
      slug: slugEdited ? prev.slug : deriveSlug(name),
    }));
  }

  function handleSlugChange(slug: string) {
    setSlugEdited(true);
    setForm((prev) => ({ ...prev, slug }));
  }

  async function handleSave() {
    setFormError(null);
    const err = validateForm(form);
    if (err) { setFormError(err); return; }

    const args = form.transport === 'stdio' && form.argsText.trim()
      ? form.argsText.trim().split(/\s+/)
      : null;

    const request: McpConnectorUpsertRequest = {
      connectorId: form.connectorId,
      name: form.name.trim(),
      slug: form.slug.trim(),
      summary: form.summary.trim(),
      transport: form.transport,
      command: form.transport === 'stdio' ? form.command.trim() || null : null,
      args,
      url: form.transport === 'sse' ? form.url.trim() || null : null,
      enabled: form.enabled,
    };

    setSaving(true);
    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        const saved = await invoke<McpConnectorView>('mcp_connector_upsert', { request });
        setConnectors((prev) => {
          const idx = prev.findIndex((c) => c.id === saved.id);
          if (idx >= 0) {
            const next = [...prev];
            next[idx] = saved;
            return next;
          }
          return [...prev, saved];
        });
      } else {
        // Mock: add/update locally
        const mock: McpConnectorView = {
          id: form.connectorId ?? `mock-${Date.now()}`,
          slug: request.slug,
          name: request.name,
          summary: request.summary,
          transport: request.transport,
          command: request.command ?? null,
          args: request.args ?? null,
          url: request.url ?? null,
          enabled: request.enabled ?? true,
          createdAt: new Date().toISOString(),
          updatedAt: new Date().toISOString(),
        };
        setConnectors((prev) => {
          const idx = prev.findIndex((c) => c.id === mock.id);
          if (idx >= 0) {
            const next = [...prev];
            next[idx] = mock;
            return next;
          }
          return [...prev, mock];
        });
      }
      setShowForm(false);
      setForm(emptyForm());
    } catch (err) {
      setFormError(`Save failed: ${err}`);
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete(connector: McpConnectorView) {
    if (confirmDeleteId !== connector.id) {
      setConfirmDeleteId(connector.id);
      return;
    }
    setConfirmDeleteId(null);
    setDeletingId(connector.id);
    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('mcp_connector_delete', { connectorId: connector.id });
      }
      setConnectors((prev) => prev.filter((c) => c.id !== connector.id));
    } catch (err) {
      setError(`Delete failed: ${err}`);
    } finally {
      setDeletingId(null);
    }
  }

  async function handleToggleEnabled(connector: McpConnectorView) {
    setTogglingId(connector.id);
    const request: McpConnectorUpsertRequest = {
      connectorId: connector.id,
      name: connector.name,
      slug: connector.slug,
      summary: connector.summary,
      transport: connector.transport,
      command: connector.command,
      args: connector.args,
      url: connector.url,
      enabled: !connector.enabled,
    };
    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        const saved = await invoke<McpConnectorView>('mcp_connector_upsert', { request });
        setConnectors((prev) => prev.map((c) => (c.id === saved.id ? saved : c)));
      } else {
        setConnectors((prev) =>
          prev.map((c) => (c.id === connector.id ? { ...c, enabled: !c.enabled } : c))
        );
      }
    } catch (err) {
      setError(`Update failed: ${err}`);
    } finally {
      setTogglingId(null);
    }
  }

  return (
    <div>
      <h2 style={headingStyle}>MCP Connectors</h2>
      <p style={subTextStyle}>
        Manage Model Context Protocol connectors available to agents. Stdio connectors run a local
        subprocess; SSE connectors stream from a remote URL.
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

      {/* Toolbar */}
      {!showForm && (
        <div style={{ marginBottom: '16px', display: 'flex', gap: '12px' }}>
          <button style={btnStyle} onClick={openAdd}>
            + Add Connector
          </button>
          <button style={cancelBtnStyle} onClick={loadConnectors} disabled={loading}>
            {loading ? 'Loading...' : '↻ Refresh'}
          </button>
        </div>
      )}

      {/* Add / Edit form */}
      {showForm && (
        <div style={formCardStyle}>
          <div style={{ marginBottom: '16px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span style={{ ...labelStyle, marginBottom: 0 }}>
              {form.connectorId ? 'Edit Connector' : 'Add Connector'}
            </span>
          </div>

          {/* Name */}
          <div style={fieldRowStyle}>
            <label style={fieldLabelStyle}>Name *</label>
            <input
              style={inputStyle}
              value={form.name}
              onChange={(e) => handleNameChange(e.target.value)}
              placeholder="e.g. GitHub Tools"
              maxLength={80}
            />
          </div>

          {/* Slug */}
          <div style={fieldRowStyle}>
            <label style={fieldLabelStyle}>Slug *</label>
            <input
              style={inputStyle}
              value={form.slug}
              onChange={(e) => handleSlugChange(e.target.value)}
              placeholder="e.g. github-tools"
              maxLength={MAX_SLUG_LEN}
            />
            <span style={fieldHintStyle}>[a-z0-9_-], max {MAX_SLUG_LEN} chars</span>
          </div>

          {/* Summary */}
          <div style={fieldRowStyle}>
            <label style={fieldLabelStyle}>Summary</label>
            <input
              style={inputStyle}
              value={form.summary}
              onChange={(e) => setForm((p) => ({ ...p, summary: e.target.value }))}
              placeholder="Short description of what this connector provides"
              maxLength={200}
            />
          </div>

          {/* Transport */}
          <div style={fieldRowStyle}>
            <label style={fieldLabelStyle}>Transport *</label>
            <select
              style={{ ...inputStyle, cursor: 'pointer' }}
              value={form.transport}
              onChange={(e) =>
                setForm((p) => ({ ...p, transport: e.target.value as 'stdio' | 'sse' }))
              }
            >
              <option value="stdio">stdio (local subprocess)</option>
              <option value="sse">sse (remote stream)</option>
            </select>
          </div>

          {/* stdio fields */}
          {form.transport === 'stdio' && (
            <>
              <div style={fieldRowStyle}>
                <label style={fieldLabelStyle}>Command *</label>
                <input
                  style={inputStyle}
                  value={form.command}
                  onChange={(e) => setForm((p) => ({ ...p, command: e.target.value }))}
                  placeholder="e.g. npx or uvx"
                />
              </div>
              <div style={fieldRowStyle}>
                <label style={fieldLabelStyle}>Arguments</label>
                <input
                  style={inputStyle}
                  value={form.argsText}
                  onChange={(e) => setForm((p) => ({ ...p, argsText: e.target.value }))}
                  placeholder="Space-separated, e.g. -y @modelcontextprotocol/server-github"
                />
                <span style={fieldHintStyle}>Split on spaces; no shell metacharacters</span>
              </div>
            </>
          )}

          {/* SSE fields */}
          {form.transport === 'sse' && (
            <div style={fieldRowStyle}>
              <label style={fieldLabelStyle}>URL *</label>
              <input
                style={inputStyle}
                value={form.url}
                onChange={(e) => setForm((p) => ({ ...p, url: e.target.value }))}
                placeholder="https://example.com/mcp/sse"
              />
            </div>
          )}

          {/* Enabled toggle */}
          <div style={{ ...fieldRowStyle, alignItems: 'center' }}>
            <label style={fieldLabelStyle}>Enabled</label>
            <button
              onClick={() => setForm((p) => ({ ...p, enabled: !p.enabled }))}
              role="switch"
              aria-checked={form.enabled}
              style={toggleStyle(form.enabled)}
            >
              <div style={toggleKnobStyle(form.enabled)} />
            </button>
          </div>

          {/* Form error */}
          {formError && <div style={formErrorStyle}>{formError}</div>}

          {/* Form actions */}
          <div style={{ display: 'flex', gap: '12px', marginTop: '20px' }}>
            <button style={btnStyle} onClick={handleSave} disabled={saving}>
              {saving ? 'Saving...' : form.connectorId ? 'Save Changes' : 'Add Connector'}
            </button>
            <button style={cancelBtnStyle} onClick={cancelForm} disabled={saving}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Connector list */}
      {loading ? (
        <div style={emptyStateStyle}>Loading connectors...</div>
      ) : connectors.length === 0 ? (
        <div style={emptyStateStyle}>
          No MCP connectors configured. Add one to extend agent capabilities.
        </div>
      ) : (
        <table style={tableStyle}>
          <thead>
            <tr>
              {(['Name', 'Slug', 'Transport', 'Status', ''] as const).map((h) => (
                <th key={h} style={thStyle}>
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {connectors.map((connector) => (
              <tr key={connector.id} style={trStyle}>
                {/* Name + summary */}
                <td style={tdStyle}>
                  <span style={{ color: 'rgb(74 222 128)', fontWeight: 600 }}>{connector.name}</span>
                  {connector.summary && (
                    <div style={{ color: 'rgb(34 197 94 / 0.5)', fontSize: '0.6rem', marginTop: '2px' }}>
                      {connector.summary}
                    </div>
                  )}
                </td>

                {/* Slug */}
                <td style={tdStyle}>
                  <code style={codeStyle}>{connector.slug}</code>
                </td>

                {/* Transport */}
                <td style={tdStyle}>
                  <code style={codeStyle}>{connector.transport}</code>
                  {connector.transport === 'stdio' && connector.command && (
                    <div style={{ color: 'rgb(34 197 94 / 0.4)', fontSize: '0.58rem', marginTop: '2px' }}>
                      {connector.command}
                      {connector.args && connector.args.length > 0 ? ' ' + connector.args.join(' ') : ''}
                    </div>
                  )}
                  {connector.transport === 'sse' && connector.url && (
                    <div style={{ color: 'rgb(34 197 94 / 0.4)', fontSize: '0.58rem', marginTop: '2px' }}>
                      {connector.url}
                    </div>
                  )}
                </td>

                {/* Status — toggle */}
                <td style={{ ...tdStyle, textAlign: 'center' }}>
                  <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: '4px' }}>
                    <button
                      onClick={() => handleToggleEnabled(connector)}
                      role="switch"
                      aria-checked={connector.enabled}
                      style={toggleStyle(connector.enabled)}
                      disabled={togglingId === connector.id}
                    >
                      <div style={toggleKnobStyle(connector.enabled)} />
                    </button>
                    <span style={{
                      fontFamily: 'monospace',
                      fontSize: '0.55rem',
                      color: connector.enabled ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.3)',
                      letterSpacing: '0.05em',
                    }}>
                      {connector.enabled ? 'enabled' : 'disabled'}
                    </span>
                  </div>
                </td>

                {/* Actions */}
                <td style={{ ...tdStyle, textAlign: 'right' }}>
                  <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end' }}>
                    <button style={actionBtnStyle} onClick={() => openEdit(connector)}>
                      Edit
                    </button>
                    <button
                      style={{
                        ...actionBtnStyle,
                        color: confirmDeleteId === connector.id
                          ? 'rgb(239 68 68)'
                          : 'rgb(239 68 68 / 0.7)',
                        borderColor: confirmDeleteId === connector.id
                          ? 'rgb(239 68 68 / 0.6)'
                          : 'rgb(239 68 68 / 0.3)',
                      }}
                      onClick={() => handleDelete(connector)}
                      disabled={deletingId === connector.id}
                    >
                      {deletingId === connector.id
                        ? '...'
                        : confirmDeleteId === connector.id
                          ? 'Confirm?'
                          : 'Delete'}
                    </button>
                  </div>
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

const labelStyle: React.CSSProperties = {
  display: 'block',
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  fontWeight: 700,
  color: 'rgb(34 197 94 / 0.7)',
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  marginBottom: '10px',
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

const cancelBtnStyle: React.CSSProperties = {
  ...btnStyle,
  color: 'rgb(34 197 94 / 0.5)',
  borderColor: 'rgb(34 197 94 / 0.2)',
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

const formCardStyle: React.CSSProperties = {
  border: '1px solid rgb(34 197 94 / 0.2)',
  borderRadius: '4px',
  padding: '20px 24px',
  marginBottom: '24px',
  backgroundColor: 'rgb(34 197 94 / 0.03)',
};

const fieldRowStyle: React.CSSProperties = {
  marginBottom: '14px',
  display: 'flex',
  flexDirection: 'column',
  gap: '4px',
};

const fieldLabelStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.65rem',
  fontWeight: 700,
  color: 'rgb(34 197 94 / 0.6)',
  letterSpacing: '0.07em',
  textTransform: 'uppercase',
};

const fieldHintStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.6rem',
  color: 'rgb(34 197 94 / 0.35)',
};

const inputStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(74 222 128)',
  backgroundColor: 'transparent',
  border: '1px solid rgb(34 197 94 / 0.3)',
  borderRadius: '3px',
  padding: '6px 10px',
  outline: 'none',
  width: '100%',
  boxSizing: 'border-box',
};

const emptyStateStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(34 197 94 / 0.4)',
  padding: '20px 0',
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

const formErrorStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.65rem',
  color: 'rgb(239 68 68 / 0.8)',
  marginTop: '8px',
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

function toggleStyle(on: boolean): React.CSSProperties {
  return {
    width: '36px',
    height: '20px',
    borderRadius: '10px',
    border: 'none',
    backgroundColor: on ? 'rgb(34 197 94 / 0.6)' : 'rgb(34 197 94 / 0.15)',
    cursor: 'pointer',
    position: 'relative',
    transition: 'background-color 0.15s ease',
    flexShrink: 0,
  };
}

function toggleKnobStyle(on: boolean): React.CSSProperties {
  return {
    position: 'absolute',
    top: '2px',
    left: on ? '18px' : '2px',
    width: '16px',
    height: '16px',
    borderRadius: '50%',
    backgroundColor: on ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.3)',
    transition: 'left 0.15s ease, background-color 0.15s ease',
  };
}
