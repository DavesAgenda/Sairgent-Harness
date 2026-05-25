import { useState, useEffect, useRef, useCallback } from 'react';
import { isTauriRuntime } from '../../sim/platform';

interface ModelSelectProps {
  provider: string;
  value: string;
  onChange: (modelId: string) => void;
  placeholder?: string;
  disabled?: boolean;
}

const MOCK_MODELS: Record<string, string[]> = {
  anthropic: [
    'claude-opus-4-20250514',
    'claude-sonnet-4-20250514',
    'claude-sonnet-4-5-20250514',
    'claude-haiku-3-5-20241022',
  ],
  openai: ['gpt-4o', 'gpt-4o-mini', 'gpt-4.1', 'gpt-4.1-mini', 'o3-mini'],
  openrouter: [
    'anthropic/claude-sonnet-4-20250514',
    'openai/gpt-4o',
    'google/gemini-2.0-flash-001',
  ],
  groq: ['llama-3.3-70b-versatile', 'mixtral-8x7b-32768'],
};

export function ModelSelect({ provider, value, onChange, placeholder, disabled }: ModelSelectProps) {
  const [models, setModels] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState(value);
  const [open, setOpen] = useState(false);
  const [focusIndex, setFocusIndex] = useState(-1);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const fetchModels = useCallback(async (slug: string) => {
    if (!slug) return;
    setLoading(true);
    setError(null);
    setModels([]);
    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        const result = await invoke<string[]>('provider_discover_models', { slug });
        setModels(result);
      } else {
        // Mock mode
        await new Promise((r) => setTimeout(r, 300));
        setModels(MOCK_MODELS[slug] ?? []);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('[ModelSelect] Discovery failed:', msg);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchModels(provider);
  }, [provider, fetchModels]);

  // Sync external value changes
  useEffect(() => {
    setSearch(value);
  }, [value]);

  const filtered = search
    ? models.filter((m) => m.toLowerCase().includes(search.toLowerCase()))
    : models;

  function handleSelect(modelId: string) {
    setSearch(modelId);
    setOpen(false);
    setFocusIndex(-1);
    onChange(modelId);
  }

  function handleInputChange(val: string) {
    setSearch(val);
    setOpen(true);
    setFocusIndex(-1);
  }

  function handleBlur(e: React.FocusEvent) {
    // If focus moved within our container, don't close
    if (containerRef.current?.contains(e.relatedTarget as Node)) return;
    setOpen(false);
    // Commit whatever is typed
    if (search !== value) {
      onChange(search);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (!open && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      setOpen(true);
      return;
    }
    if (!open) return;

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setFocusIndex((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setFocusIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (focusIndex >= 0 && focusIndex < filtered.length) {
        const item = filtered[focusIndex];
        if (item) handleSelect(item);
      } else if (search) {
        handleSelect(search);
      }
    } else if (e.key === 'Escape') {
      setOpen(false);
      setFocusIndex(-1);
    }
  }

  // Scroll focused item into view
  useEffect(() => {
    if (focusIndex >= 0 && listRef.current) {
      const items = listRef.current.children;
      if (items[focusIndex]) {
        (items[focusIndex] as HTMLElement).scrollIntoView({ block: 'nearest' });
      }
    }
  }, [focusIndex]);

  // If error, fall back to plain text input
  if (error && models.length === 0) {
    return (
      <div>
        <input
          type="text"
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
          }}
          onBlur={() => {
            if (search !== value) onChange(search);
          }}
          placeholder={placeholder ?? 'Type model name'}
          disabled={disabled}
          style={inputStyle}
        />
        <div style={errorStyle}>
          Could not load models. <button onClick={() => fetchModels(provider)} style={retryBtnStyle}>Retry</button>
        </div>
      </div>
    );
  }

  return (
    <div ref={containerRef} style={{ position: 'relative' }} onBlur={handleBlur}>
      <div style={{ position: 'relative' }}>
        <input
          ref={inputRef}
          type="text"
          value={search}
          onChange={(e) => handleInputChange(e.target.value)}
          onFocus={() => { if (models.length > 0) setOpen(true); }}
          onKeyDown={handleKeyDown}
          placeholder={loading ? 'Loading models...' : (placeholder ?? 'Search models')}
          disabled={disabled || loading}
          style={{
            ...inputStyle,
            paddingRight: '32px',
          }}
        />
        {loading && (
          <span style={spinnerStyle}>...</span>
        )}
      </div>

      {open && filtered.length > 0 && (
        <div ref={listRef} style={dropdownStyle}>
          {filtered.slice(0, 100).map((model, i) => (
            <div
              key={model}
              tabIndex={-1}
              onMouseDown={(e) => {
                e.preventDefault(); // Prevent blur before select
                handleSelect(model);
              }}
              onMouseEnter={() => setFocusIndex(i)}
              style={{
                ...itemStyle,
                backgroundColor: i === focusIndex ? 'rgb(34 197 94 / 0.15)' : 'transparent',
              }}
            >
              {model}
            </div>
          ))}
          {filtered.length > 100 && (
            <div style={{ ...itemStyle, color: 'rgb(34 197 94 / 0.4)', fontStyle: 'italic' }}>
              {filtered.length - 100} more -- refine your search
            </div>
          )}
        </div>
      )}

      {open && filtered.length === 0 && !loading && models.length > 0 && (
        <div style={dropdownStyle}>
          <div style={{ ...itemStyle, color: 'rgb(34 197 94 / 0.4)' }}>
            No matching models
          </div>
        </div>
      )}
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.75rem',
  color: 'rgb(74 222 128)',
  backgroundColor: 'rgb(9 9 11)',
  border: '1px solid rgb(34 197 94 / 0.4)',
  padding: '8px 12px',
  borderRadius: '4px',
  width: '100%',
  boxSizing: 'border-box',
};

const dropdownStyle: React.CSSProperties = {
  position: 'absolute',
  top: '100%',
  left: 0,
  right: 0,
  maxHeight: '200px',
  overflowY: 'auto',
  backgroundColor: 'rgb(9 9 11)',
  border: '1px solid rgb(34 197 94 / 0.4)',
  borderTop: 'none',
  borderRadius: '0 0 4px 4px',
  zIndex: 100,
};

const itemStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(74 222 128)',
  padding: '6px 12px',
  cursor: 'pointer',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
};

const errorStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.65rem',
  color: 'rgb(239 68 68 / 0.7)',
  marginTop: '4px',
};

const retryBtnStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.65rem',
  color: 'rgb(74 222 128)',
  background: 'none',
  border: 'none',
  cursor: 'pointer',
  textDecoration: 'underline',
  padding: 0,
};

const spinnerStyle: React.CSSProperties = {
  position: 'absolute',
  right: '10px',
  top: '50%',
  transform: 'translateY(-50%)',
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(34 197 94 / 0.5)',
  animation: 'pulse 1s ease-in-out infinite',
};
