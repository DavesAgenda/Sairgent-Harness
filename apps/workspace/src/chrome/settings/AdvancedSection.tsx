import { useState } from 'react';
import { isTauriRuntime } from '../../sim/platform';

export function AdvancedSection() {
  const [debugLog, setDebugLog] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<string | null>(null);

  async function checkEngineStatus() {
    if (isTauriRuntime()) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const hasSecrets = await invoke<boolean>('secrets_status');
        setConnectionStatus(hasSecrets ? 'Engine credentials found' : 'No credentials configured');
      } catch (err) {
        setConnectionStatus(`Engine check failed: ${err}`);
      }
    } else {
      setConnectionStatus('Running in mock mode (no engine)');
    }
  }

  return (
    <div>
      <h2 style={headingStyle}>Advanced</h2>
      <p style={subTextStyle}>
        Diagnostics and debugging tools for your workspace.
      </p>

      {/* Engine diagnostics */}
      <div style={{ marginBottom: '28px' }}>
        <label style={labelStyle}>Engine Diagnostics</label>
        <div
          style={{
            padding: '12px 16px',
            border: '1px solid rgb(34 197 94 / 0.15)',
            borderRadius: '4px',
            marginBottom: '12px',
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '8px' }}>
            <div
              style={{
                width: '6px',
                height: '6px',
                borderRadius: '50%',
                backgroundColor: isTauriRuntime() ? 'rgb(74 222 128)' : '#eab308',
              }}
            />
            <span style={itemTextStyle}>
              Mode: {isTauriRuntime() ? 'Tauri (native)' : 'Browser (mock)'}
            </span>
          </div>
          {connectionStatus && (
            <div style={{ ...itemTextStyle, color: 'rgb(34 197 94 / 0.5)', marginTop: '4px' }}>
              {connectionStatus}
            </div>
          )}
        </div>
        <button style={btnStyle} onClick={checkEngineStatus}>
          Check Engine Status
        </button>
      </div>

      {/* Debug log toggle */}
      <div style={{ marginBottom: '28px' }}>
        <label style={labelStyle}>Debug Log</label>
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
          <span style={itemTextStyle}>Show verbose logging in developer console</span>
          <button
            onClick={() => setDebugLog(!debugLog)}
            role="switch"
            aria-checked={debugLog}
            style={{
              width: '36px',
              height: '20px',
              borderRadius: '10px',
              border: 'none',
              backgroundColor: debugLog ? 'rgb(34 197 94 / 0.6)' : 'rgb(34 197 94 / 0.15)',
              cursor: 'pointer',
              position: 'relative',
              transition: 'background-color 0.15s ease',
              flexShrink: 0,
            }}
          >
            <div
              style={{
                position: 'absolute',
                top: '2px',
                left: debugLog ? '18px' : '2px',
                width: '16px',
                height: '16px',
                borderRadius: '50%',
                backgroundColor: debugLog ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.3)',
                transition: 'left 0.15s ease, background-color 0.15s ease',
              }}
            />
          </button>
        </div>
      </div>

      {/* Danger zone */}
      <div style={{ marginBottom: '28px', paddingTop: '16px', borderTop: '1px solid rgb(34 197 94 / 0.1)' }}>
        <label style={{ ...labelStyle, color: '#ef4444 / 0.7' }}>Danger Zone</label>
        <p
          style={{
            fontFamily: 'monospace',
            fontSize: '0.6rem',
            color: 'rgb(34 197 94 / 0.4)',
            marginBottom: '12px',
            lineHeight: 1.5,
          }}
        >
          Reset your workspace to default state. This clears local preferences only -- your team data and engine state are not affected.
        </p>
        <button
          style={{
            ...btnStyle,
            borderColor: '#ef4444 / 0.4',
            color: '#ef4444',
          }}
          onClick={() => {
            if (confirm('Reset all workspace preferences? This cannot be undone.')) {
              localStorage.clear();
              window.location.reload();
            }
          }}
        >
          Reset Workspace
        </button>
      </div>
    </div>
  );
}

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

const itemTextStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(74 222 128)',
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
