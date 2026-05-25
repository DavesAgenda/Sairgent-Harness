import { useState, useEffect } from 'react';
import { isTauriRuntime } from '../../sim/platform';
import { ConnectionWizard } from './ConnectionWizard';
import { ModelSelect } from './ModelSelect';

interface ProviderConnection {
  slug: string;
  label: string;
  envVar: string;
  hasSecret: boolean;
  enabled: boolean;
}

interface SettingsData {
  defaultLlmProvider: string;
  defaultLlmModel: string;
  llmCredentials: ProviderConnection[];
}

const ENV_VARS: Record<string, string> = {
  anthropic: 'ANTHROPIC_API_KEY',
  openai: 'OPENAI_API_KEY',
  openrouter: 'OPENROUTER_API_KEY',
  groq: 'GROQ_API_KEY',
};

const LOCAL_STORAGE_KEY = 'sairgent_mock_connections';
const LOCAL_STORAGE_PROVIDER_KEY = 'sairgent_mock_default_provider';
const LOCAL_STORAGE_MODEL_KEY = 'sairgent_mock_default_model';

function loadMockConnections(): ProviderConnection[] {
  try {
    const raw = localStorage.getItem(LOCAL_STORAGE_KEY);
    if (raw) return JSON.parse(raw);
  } catch { /* ignore */ }
  return defaultProviders();
}

function saveMockConnections(connections: ProviderConnection[]) {
  localStorage.setItem(LOCAL_STORAGE_KEY, JSON.stringify(connections));
}

function defaultProviders(): ProviderConnection[] {
  return [
    { slug: 'anthropic', label: 'Anthropic', envVar: 'ANTHROPIC_API_KEY', hasSecret: false, enabled: false },
    { slug: 'openai', label: 'OpenAI', envVar: 'OPENAI_API_KEY', hasSecret: false, enabled: false },
    { slug: 'openrouter', label: 'OpenRouter', envVar: 'OPENROUTER_API_KEY', hasSecret: false, enabled: false },
    { slug: 'groq', label: 'Groq', envVar: 'GROQ_API_KEY', hasSecret: false, enabled: false },
  ];
}

export function ConnectionsSection({ onConnectionSaved }: { onConnectionSaved?: () => void }) {
  const [connections, setConnections] = useState<ProviderConnection[]>([]);
  const [defaultProvider, setDefaultProvider] = useState('anthropic');
  const [defaultModel, setDefaultModel] = useState('');
  const [wizardOpen, setWizardOpen] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    loadSettings();
  }, []);

  async function loadSettings() {
    setLoading(true);
    if (isTauriRuntime()) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const settings = await invoke<SettingsData & { llmCredentials: { slug: string; label: string; envVar: string; hasSecret: boolean; enabled: boolean }[] }>('settings_load');
        setConnections(
          settings.llmCredentials.map((c) => ({
            slug: c.slug,
            label: c.label,
            envVar: c.envVar || ENV_VARS[c.slug] || '',
            hasSecret: c.hasSecret,
            enabled: c.enabled,
          })),
        );
        setDefaultProvider(settings.defaultLlmProvider || 'anthropic');
        setDefaultModel(settings.defaultLlmModel || '');
      } catch (err) {
        console.error('[settings] Failed to load:', err);
        setConnections(defaultProviders());
      }
    } else {
      setConnections(loadMockConnections());
      setDefaultProvider(localStorage.getItem(LOCAL_STORAGE_PROVIDER_KEY) || 'anthropic');
      setDefaultModel(localStorage.getItem(LOCAL_STORAGE_MODEL_KEY) || '');
    }
    setLoading(false);
  }

  // Track whether user has unsaved changes
  const [savedProvider, setSavedProvider] = useState(defaultProvider);
  const [savedModel, setSavedModel] = useState(defaultModel);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const isDirty = defaultProvider !== savedProvider || defaultModel !== savedModel;

  function handleProviderChange(provider: string) {
    setDefaultProvider(provider);
    setSaveSuccess(false);
  }

  function handleModelChange(model: string) {
    setDefaultModel(model);
    setSaveSuccess(false);
  }

  async function handleSave() {
    setSaving(true);
    setSaveSuccess(false);
    if (isTauriRuntime()) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('settings_save', {
          request: {
            defaultLlmProvider: defaultProvider,
            defaultLlmModel: defaultModel || '',
            llmCredentials: connections.map((c) => ({
              slug: c.slug,
              label: c.label,
              envVar: c.envVar || ENV_VARS[c.slug] || '',
              enabled: c.enabled,
            })),
            toolCredentials: [],
          },
        });
        setSavedProvider(defaultProvider);
        setSavedModel(defaultModel);
        setSaveSuccess(true);
        onConnectionSaved?.();
      } catch (err) {
        console.error('[settings] Failed to save provider/model:', err);
      }
    } else {
      localStorage.setItem(LOCAL_STORAGE_PROVIDER_KEY, defaultProvider);
      localStorage.setItem(LOCAL_STORAGE_MODEL_KEY, defaultModel);
      setSavedProvider(defaultProvider);
      setSavedModel(defaultModel);
      setSaveSuccess(true);
    }
    setSaving(false);
  }

  function handleWizardComplete(provider: string, _key: string) {
    if (isTauriRuntime()) {
      loadSettings();
      onConnectionSaved?.();
    } else {
      const updated = connections.map((c) =>
        c.slug === provider ? { ...c, hasSecret: true, enabled: true } : c,
      );
      setConnections(updated);
      saveMockConnections(updated);
    }
    setWizardOpen(false);
  }

  const connectedCount = connections.filter((c) => c.hasSecret).length;
  const connectedProviders = connections.filter((c) => c.hasSecret);

  if (loading) {
    return (
      <div style={{ color: 'rgb(34 197 94 / 0.5)', fontFamily: 'monospace', fontSize: '0.75rem' }}>
        Loading connections...
      </div>
    );
  }

  return (
    <div>
      <h2 style={sectionHeadingStyle}>Connections</h2>
      <p style={sectionDescStyle}>
        Connect your AI providers so your team can think.{' '}
        {connectedCount > 0
          ? `${connectedCount} provider${connectedCount === 1 ? '' : 's'} connected.`
          : 'No providers connected yet.'}
      </p>

      {/* Default provider selector */}
      {connectedCount > 0 && (
        <div style={{ marginBottom: '24px', padding: '16px', border: '1px solid rgb(34 197 94 / 0.3)', borderRadius: '4px', backgroundColor: 'rgb(34 197 94 / 0.03)' }}>
          <label style={labelStyle}>Active Provider</label>
          <p style={{ ...sectionDescStyle, marginBottom: '8px' }}>
            Which AI provider should your team use for thinking?
          </p>
          <select
            value={defaultProvider}
            onChange={(e) => handleProviderChange(e.target.value)}
            disabled={saving}
            style={selectStyle}
          >
            {connectedProviders.length > 0
              ? connectedProviders.map((c) => (
                  <option key={c.slug} value={c.slug}>{c.label}</option>
                ))
              : connections.map((c) => (
                  <option key={c.slug} value={c.slug}>{c.label}{!c.hasSecret ? ' (no key)' : ''}</option>
                ))
            }
          </select>

          <label style={{ ...labelStyle, marginTop: '16px', display: 'block' }}>Model</label>
          <p style={{ ...sectionDescStyle, marginBottom: '8px' }}>
            Leave blank for the provider's default model.
          </p>
          <ModelSelect
            provider={defaultProvider}
            value={defaultModel}
            onChange={handleModelChange}
            placeholder={modelPlaceholder(defaultProvider)}
            disabled={saving}
          />

          {/* Save button */}
          <div style={{ marginTop: '16px', display: 'flex', alignItems: 'center', gap: '12px' }}>
            <button
              onClick={handleSave}
              disabled={saving || !isDirty}
              style={{
                fontFamily: 'monospace', fontSize: '0.7rem', letterSpacing: '0.1em',
                textTransform: 'uppercase',
                color: isDirty ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.3)',
                backgroundColor: isDirty ? 'rgb(34 197 94 / 0.1)' : 'transparent',
                border: `1px solid ${isDirty ? 'rgb(34 197 94 / 0.6)' : 'rgb(34 197 94 / 0.15)'}`,
                padding: '8px 24px', cursor: isDirty ? 'pointer' : 'default',
                transition: 'all 0.15s ease',
              }}
            >
              {saving ? 'Saving...' : 'Save'}
            </button>
            {saveSuccess && (
              <span style={{ fontFamily: 'monospace', fontSize: '0.65rem', color: 'rgb(74 222 128)' }}>
                ✓ Saved
              </span>
            )}
          </div>
        </div>
      )}

      {/* Connection cards */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))', gap: '12px', marginBottom: '24px' }}>
        {connections.map((conn) => (
          <div
            key={conn.slug}
            style={{
              border: `1px solid ${conn.hasSecret ? 'rgb(34 197 94 / 0.5)' : 'rgb(34 197 94 / 0.15)'}`,
              borderRadius: '4px',
              padding: '16px',
              backgroundColor: conn.hasSecret ? 'rgb(34 197 94 / 0.04)' : 'transparent',
            }}
          >
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '8px' }}>
              <span style={{ fontFamily: 'monospace', fontSize: '0.75rem', fontWeight: 700, color: 'rgb(74 222 128)' }}>
                {conn.label}
              </span>
              <span
                style={{
                  fontFamily: 'monospace', fontSize: '0.6rem',
                  color: conn.hasSecret ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.3)',
                  letterSpacing: '0.08em', textTransform: 'uppercase',
                }}
              >
                {conn.hasSecret ? '● Connected' : '○ Not set'}
              </span>
            </div>
            {conn.slug === defaultProvider && conn.hasSecret && (
              <span style={{ fontFamily: 'monospace', fontSize: '0.55rem', color: 'rgb(34 197 94 / 0.5)', letterSpacing: '0.1em', textTransform: 'uppercase' }}>
                Active
              </span>
            )}
          </div>
        ))}
      </div>

      {/* Add connection button */}
      <button
        onClick={() => setWizardOpen(true)}
        style={{
          fontFamily: 'monospace', fontSize: '0.7rem', letterSpacing: '0.1em',
          textTransform: 'uppercase', color: 'rgb(74 222 128)',
          backgroundColor: 'transparent', border: '1px solid rgb(34 197 94 / 0.5)',
          padding: '8px 20px', cursor: 'pointer', transition: 'all 0.15s ease',
        }}
      >
        + Add Connection
      </button>

      {wizardOpen && (
        <ConnectionWizard
          onComplete={handleWizardComplete}
          onCancel={() => setWizardOpen(false)}
        />
      )}
    </div>
  );
}

function modelPlaceholder(provider: string): string {
  switch (provider) {
    case 'anthropic': return 'e.g. claude-sonnet-4-5-20250514';
    case 'openai': return 'e.g. gpt-4o';
    case 'openrouter': return 'e.g. anthropic/claude-3.5-sonnet';
    case 'groq': return 'e.g. llama-3.3-70b-versatile';
    default: return 'Model name';
  }
}

const sectionHeadingStyle: React.CSSProperties = {
  fontFamily: 'monospace', fontSize: 'var(--ws-font-lg)', fontWeight: 700,
  color: 'var(--ws-fg-primary)', letterSpacing: '0.1em',
  textTransform: 'uppercase', marginBottom: 'var(--ws-space-sm)',
};

const sectionDescStyle: React.CSSProperties = {
  fontFamily: 'monospace', fontSize: 'var(--ws-font-sm)',
  color: 'var(--ws-fg-secondary)', marginBottom: 'var(--ws-space-xl)', lineHeight: 1.5,
};

const labelStyle: React.CSSProperties = {
  fontFamily: 'monospace', fontSize: 'var(--ws-font-sm)', fontWeight: 700,
  color: 'var(--ws-fg-primary)', letterSpacing: '0.08em',
  textTransform: 'uppercase', marginBottom: 'var(--ws-space-xs)', display: 'block',
};

const selectStyle: React.CSSProperties = {
  fontFamily: 'monospace', fontSize: 'var(--ws-font-base)',
  color: 'var(--ws-fg-primary)', backgroundColor: 'var(--ws-bg)',
  border: '1px solid var(--ws-border)', padding: 'var(--ws-space-sm) var(--ws-space-md)',
  borderRadius: 'var(--ws-radius-md)', width: '100%', cursor: 'pointer',
};

