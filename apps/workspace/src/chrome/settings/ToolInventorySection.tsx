import { useState, useEffect } from 'react';
import { isTauriRuntime } from '../../sim/platform';
import type { CliTool, CliToolUpsertRequest } from '../../types';

const ALLOWED_COMMANDS = ['npx', 'uvx', 'node', 'python3', 'python', 'deno'] as const;
type AllowedCommand = (typeof ALLOWED_COMMANDS)[number];

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
  id?: string;
  name: string;
  slug: string;
  summary: string;
  command: AllowedCommand;
  argsText: string;
  cwd: string;
  enabled: boolean;
}

const emptyForm = (): FormState => ({
  id: undefined,
  name: '',
  slug: '',
  summary: '',
  command: 'npx',
  argsText: '',
  cwd: '',
  enabled: true,
});

function validateSlug(slug: string): string | null {
  if (!slug) return 'Slug is required';
  if (slug.length > MAX_SLUG_LEN) return `Slug must be ${MAX_SLUG_LEN} characters or fewer`;
  if (!SLUG_RE.test(slug)) return 'Slug must only contain lowercase letters, numbers, - or _';
  return null;
}

export function ToolInventorySection() {
  const [tools, setTools] = useState<CliTool[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<FormState>(emptyForm());
  const [slugEdited, setSlugEdited] = useState(false);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  useEffect(() => {
    loadTools();
  }, []);

  async function loadTools() {
    setLoading(true);
    setError(null);
    if (isTauriRuntime()) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const result = await invoke<CliTool[]>('cli_tools_list');
        setTools(result);
      } catch (err) {
        setError(`Failed to load CLI tools: ${err}`);
      }
    } else {
      // Mock mode: show empty list
      setTools([]);
    }
    setLoading(false);
  }

  function openAdd() {
    setForm(emptyForm());
    setSlugEdited(false);
    setFormError(null);
    setShowForm(true);
  }

  function openEdit(tool: CliTool) {
    const argsText = tool.args ? tool.args.join(' ') : '';
    setForm({
      id: tool.id,
      name: tool.name,
      slug: tool.slug,
      summary: tool.summary ?? '',
      command: (ALLOWED_COMMANDS as readonly string[]).includes(tool.command)
        ? (tool.command as AllowedCommand)
        : 'npx',
      argsText,
      cwd: tool.cwd ?? '',
      enabled: tool.enabled,
    });
    setSlugEdited(true);
    setFormError(null);
    setShowForm(true);
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

    const slugErr = validateSlug(form.slug);
    if (slugErr) { setFormError(slugErr); return; }
    if (!form.name.trim()) { setFormError('Name is required'); return; }
    if (!form.command) { setFormError('Command is required'); return; }

    const args = form.argsText.trim()
      ? form.argsText.trim().split(/\s+/)
      : null;

    const request: CliToolUpsertRequest = {
      id: form.id,
      name: form.name.trim(),
      slug: form.slug.trim(),
      summary: form.summary.trim() || null,
      command: form.command,
      args,
      env: null,
      cwd: form.cwd.trim() || null,
      enabled: form.enabled,
    };

    setSaving(true);
    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        const saved = await invoke<CliTool>('cli_tool_upsert', { request });
        setTools((prev) => {
          const idx = prev.findIndex((t) => t.id === saved.id);
          if (idx >= 0) {
            const next = [...prev];
            next[idx] = saved;
            return next;
          }
          return [...prev, saved];
        });
      } else {
        // Mock: add locally
        const mock: CliTool = {
          id: form.id ?? `mock-${Date.now()}`,
          ...request,
          summary: request.summary ?? null,
          args: request.args ?? null,
          env: null,
          cwd: request.cwd ?? null,
          enabled: request.enabled ?? true,
          createdAt: new Date().toISOString(),
          updatedAt: new Date().toISOString(),
        };
        setTools((prev) => {
          const idx = prev.findIndex((t) => t.id === mock.id);
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

  async function handleDelete(tool: CliTool) {
    if (!confirm(`Delete CLI tool "${tool.name}"? This cannot be undone.`)) return;
    setDeletingId(tool.id);
    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('cli_tool_delete', { toolId: tool.id });
      }
      setTools((prev) => prev.filter((t) => t.id !== tool.id));
    } catch (err) {
      setError(`Delete failed: ${err}`);
    } finally {
      setDeletingId(null);
    }
  }

  async function handleToggleEnabled(tool: CliTool) {
    const request: CliToolUpsertRequest = {
      id: tool.id,
      name: tool.name,
      slug: tool.slug,
      summary: tool.summary,
      command: tool.command,
      args: tool.args,
      env: tool.env,
      cwd: tool.cwd,
      enabled: !tool.enabled,
    };
    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        const saved = await invoke<CliTool>('cli_tool_upsert', { request });
        setTools((prev) => prev.map((t) => (t.id === saved.id ? saved : t)));
      } else {
        setTools((prev) =>
          prev.map((t) => (t.id === tool.id ? { ...t, enabled: !t.enabled } : t))
        );
      }
    } catch (err) {
      setError(`Update failed: ${err}`);
    }
  }

  return (
    <div>
      <h2 style={headingStyle}>Tool Inventory</h2>
      <p style={subTextStyle}>
        Manage CLI tools available to agents. Tools are invoked as subprocesses with
        allowlisted commands.
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
        <div style={{ marginBottom: '16px' }}>
          <button style={btnStyle} onClick={openAdd}>
            + Add Tool
          </button>
        </div>
      )}

      {/* Add / Edit form */}
      {showForm && (
        <div style={formCardStyle}>
          <div style={{ marginBottom: '16px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span style={{ ...labelStyle, marginBottom: 0 }}>
              {form.id ? 'Edit Tool' : 'Add Tool'}
            </span>
          </div>

          {/* Name */}
          <div style={fieldRowStyle}>
            <label style={fieldLabelStyle}>Name *</label>
            <input
              style={inputStyle}
              value={form.name}
              onChange={(e) => handleNameChange(e.target.value)}
              placeholder="e.g. Context7 Docs"
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
              placeholder="e.g. context7-docs"
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
              placeholder="Short description (optional)"
              maxLength={200}
            />
          </div>

          {/* Command */}
          <div style={fieldRowStyle}>
            <label style={fieldLabelStyle}>Command *</label>
            <select
              style={{ ...inputStyle, cursor: 'pointer' }}
              value={form.command}
              onChange={(e) =>
                setForm((p) => ({ ...p, command: e.target.value as AllowedCommand }))
              }
            >
              {ALLOWED_COMMANDS.map((cmd) => (
                <option key={cmd} value={cmd}>
                  {cmd}
                </option>
              ))}
            </select>
          </div>

          {/* Arguments */}
          <div style={fieldRowStyle}>
            <label style={fieldLabelStyle}>Arguments</label>
            <input
              style={inputStyle}
              value={form.argsText}
              onChange={(e) => setForm((p) => ({ ...p, argsText: e.target.value }))}
              placeholder="Space-separated, e.g. -y @context7/mcp-server"
            />
            <span style={fieldHintStyle}>Split on spaces; no shell metacharacters</span>
          </div>

          {/* Working Directory */}
          <div style={fieldRowStyle}>
            <label style={fieldLabelStyle}>Working Dir</label>
            <input
              style={inputStyle}
              value={form.cwd}
              onChange={(e) => setForm((p) => ({ ...p, cwd: e.target.value }))}
              placeholder="Absolute path (optional)"
            />
          </div>

          {/* Enabled */}
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
            <button
              style={btnStyle}
              onClick={handleSave}
              disabled={saving}
            >
              {saving ? 'Saving...' : form.id ? 'Save Changes' : 'Add Tool'}
            </button>
            <button style={cancelBtnStyle} onClick={cancelForm} disabled={saving}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Tool list */}
      {loading ? (
        <div style={emptyStateStyle}>Loading...</div>
      ) : tools.length === 0 ? (
        <div style={emptyStateStyle}>
          No CLI tools configured. Add one to make it available to agents.
        </div>
      ) : (
        <table style={tableStyle}>
          <thead>
            <tr>
              {(['Name', 'Slug', 'Command', 'Enabled', ''] as const).map((h) => (
                <th key={h} style={thStyle}>
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {tools.map((tool) => (
              <tr key={tool.id} style={trStyle}>
                <td style={tdStyle}>
                  <span style={{ color: 'rgb(74 222 128)', fontWeight: 600 }}>{tool.name}</span>
                  {tool.summary && (
                    <div style={{ color: 'rgb(34 197 94 / 0.5)', fontSize: '0.6rem', marginTop: '2px' }}>
                      {tool.summary}
                    </div>
                  )}
                </td>
                <td style={tdStyle}>
                  <code style={codeStyle}>{tool.slug}</code>
                </td>
                <td style={tdStyle}>
                  <code style={codeStyle}>{tool.command}</code>
                  {tool.args && tool.args.length > 0 && (
                    <div style={{ color: 'rgb(34 197 94 / 0.4)', fontSize: '0.6rem', marginTop: '2px' }}>
                      {tool.args.join(' ')}
                    </div>
                  )}
                </td>
                <td style={{ ...tdStyle, textAlign: 'center' }}>
                  <button
                    onClick={() => handleToggleEnabled(tool)}
                    role="switch"
                    aria-checked={tool.enabled}
                    style={toggleStyle(tool.enabled)}
                  >
                    <div style={toggleKnobStyle(tool.enabled)} />
                  </button>
                </td>
                <td style={{ ...tdStyle, textAlign: 'right' }}>
                  <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end' }}>
                    <button style={actionBtnStyle} onClick={() => openEdit(tool)}>
                      Edit
                    </button>
                    <button
                      style={{ ...actionBtnStyle, color: 'rgb(239 68 68 / 0.7)', borderColor: 'rgb(239 68 68 / 0.3)' }}
                      onClick={() => handleDelete(tool)}
                      disabled={deletingId === tool.id}
                    >
                      {deletingId === tool.id ? '...' : 'Delete'}
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
