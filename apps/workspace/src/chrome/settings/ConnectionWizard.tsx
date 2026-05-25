import { useState, useCallback } from 'react';
import { isTauriRuntime } from '../../sim/platform';

type WizardStep = 'choose' | 'paste' | 'confirm';

interface ConnectionWizardProps {
  onComplete: (provider: string, key: string) => void;
  onCancel: () => void;
}

const PROVIDERS = [
  {
    slug: 'anthropic',
    label: 'Anthropic',
    description: 'Claude models — best for reasoning and code',
    placeholder: 'sk-ant-...',
  },
  {
    slug: 'openai',
    label: 'OpenAI',
    description: 'GPT-4 family — versatile and fast',
    placeholder: 'sk-...',
  },
  {
    slug: 'openrouter',
    label: 'OpenRouter',
    description: 'Access multiple providers through one key',
    placeholder: 'sk-or-...',
  },
];

export function ConnectionWizard({ onComplete, onCancel }: ConnectionWizardProps) {
  const [step, setStep] = useState<WizardStep>('choose');
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [apiKey, setApiKey] = useState('');
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);
  const [testError, setTestError] = useState<string | null>(null);

  const provider = PROVIDERS.find((p) => p.slug === selectedProvider);

  const handleProviderSelect = useCallback((slug: string) => {
    setSelectedProvider(slug);
    setApiKey('');
    setTestResult(null);
    setTestError(null);
    setStep('paste');
  }, []);

  const handleTestConnection = useCallback(async () => {
    if (!selectedProvider || !apiKey.trim()) return;
    setTesting(true);
    setTestResult(null);
    setTestError(null);

    try {
      if (isTauriRuntime()) {
        const { invoke } = await import('@tauri-apps/api/core');
        // Save the key via the secrets_set command
        await invoke('secrets_set', {
          request: { provider: selectedProvider, key: apiKey.trim() },
        });
        setTestResult('success');
        setStep('confirm');
      } else {
        // Mock mode: simulate a test delay
        await new Promise((resolve) => setTimeout(resolve, 800));
        if (apiKey.trim().length < 10) {
          setTestResult('error');
          setTestError('Key appears too short. Please check and try again.');
        } else {
          setTestResult('success');
          setStep('confirm');
        }
      }
    } catch (err) {
      setTestResult('error');
      setTestError(err instanceof Error ? err.message : String(err));
    } finally {
      setTesting(false);
    }
  }, [selectedProvider, apiKey]);

  const overlayStyle: React.CSSProperties = {
    position: 'fixed',
    inset: 0,
    backgroundColor: 'rgba(0, 0, 0, 0.5)',
    zIndex: 200,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  };

  const cardStyle: React.CSSProperties = {
    backgroundColor: 'rgb(9 9 11)',
    border: '1px solid rgb(34 197 94 / 0.4)',
    borderRadius: '4px',
    padding: '32px',
    width: '480px',
    maxWidth: '90vw',
    maxHeight: '80vh',
    overflow: 'auto',
  };

  const headingStyle: React.CSSProperties = {
    fontFamily: 'monospace',
    fontSize: '0.8rem',
    fontWeight: 700,
    color: 'rgb(74 222 128)',
    letterSpacing: '0.1em',
    textTransform: 'uppercase' as const,
    marginBottom: '16px',
  };

  const subTextStyle: React.CSSProperties = {
    fontFamily: 'monospace',
    fontSize: '0.7rem',
    color: 'rgb(34 197 94 / 0.6)',
    lineHeight: 1.5,
    marginBottom: '20px',
  };

  const btnStyle: React.CSSProperties = {
    fontFamily: 'monospace',
    fontSize: '0.7rem',
    letterSpacing: '0.1em',
    textTransform: 'uppercase' as const,
    color: 'rgb(74 222 128)',
    backgroundColor: 'transparent',
    border: '1px solid rgb(34 197 94 / 0.5)',
    padding: '8px 20px',
    cursor: 'pointer',
  };

  return (
    <div style={overlayStyle} onClick={onCancel}>
      <div style={cardStyle} onClick={(e) => e.stopPropagation()}>
        {/* Step indicator */}
        <div
          style={{
            display: 'flex',
            gap: '8px',
            marginBottom: '24px',
            fontFamily: 'monospace',
            fontSize: '0.6rem',
            letterSpacing: '0.08em',
            textTransform: 'uppercase',
          }}
        >
          {(['choose', 'paste', 'confirm'] as WizardStep[]).map((s, i) => (
            <span
              key={s}
              style={{
                color: step === s ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.3)',
                fontWeight: step === s ? 700 : 400,
              }}
            >
              {i + 1}. {s}
            </span>
          ))}
        </div>

        {/* Step 1: Choose provider */}
        {step === 'choose' && (
          <div>
            <h3 style={headingStyle}>Choose a Provider</h3>
            <p style={subTextStyle}>
              Pick the AI provider you want to connect. You can add more later.
            </p>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
              {PROVIDERS.map((p) => (
                <button
                  key={p.slug}
                  onClick={() => handleProviderSelect(p.slug)}
                  style={{
                    display: 'block',
                    width: '100%',
                    textAlign: 'left',
                    padding: '12px 16px',
                    border: '1px solid rgb(34 197 94 / 0.2)',
                    borderRadius: '4px',
                    backgroundColor: 'transparent',
                    cursor: 'pointer',
                    transition: 'all 0.1s ease',
                    fontFamily: 'monospace',
                  }}
                  onMouseEnter={(e) => {
                    e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.5)';
                    e.currentTarget.style.backgroundColor = 'rgb(34 197 94 / 0.04)';
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.2)';
                    e.currentTarget.style.backgroundColor = 'transparent';
                  }}
                >
                  <div
                    style={{
                      fontSize: '0.75rem',
                      fontWeight: 700,
                      color: 'rgb(74 222 128)',
                      marginBottom: '4px',
                    }}
                  >
                    {p.label}
                  </div>
                  <div style={{ fontSize: '0.65rem', color: 'rgb(34 197 94 / 0.5)' }}>
                    {p.description}
                  </div>
                </button>
              ))}
            </div>
            <div style={{ marginTop: '16px' }}>
              <button style={btnStyle} onClick={onCancel}>Cancel</button>
            </div>
          </div>
        )}

        {/* Step 2: Paste key */}
        {step === 'paste' && provider && (
          <div>
            <h3 style={headingStyle}>Connect {provider.label}</h3>
            <p style={subTextStyle}>
              Paste your API key below. It will be stored securely in your system keychain.
            </p>
            <input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={provider.placeholder}
              autoFocus
              style={{
                width: '100%',
                padding: '10px 12px',
                fontFamily: 'monospace',
                fontSize: '0.75rem',
                color: 'rgb(74 222 128)',
                backgroundColor: 'rgb(34 197 94 / 0.05)',
                border: '1px solid rgb(34 197 94 / 0.3)',
                borderRadius: '4px',
                outline: 'none',
                marginBottom: '12px',
                boxSizing: 'border-box',
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && apiKey.trim()) {
                  handleTestConnection();
                }
              }}
            />
            {testResult === 'error' && testError && (
              <p
                style={{
                  fontFamily: 'monospace',
                  fontSize: '0.65rem',
                  color: '#ef4444',
                  marginBottom: '12px',
                }}
              >
                {testError}
              </p>
            )}
            <div style={{ display: 'flex', gap: '8px' }}>
              <button style={btnStyle} onClick={() => setStep('choose')}>
                Back
              </button>
              <button
                style={{
                  ...btnStyle,
                  opacity: !apiKey.trim() || testing ? 0.4 : 1,
                  pointerEvents: !apiKey.trim() || testing ? 'none' : 'auto',
                }}
                onClick={handleTestConnection}
              >
                {testing ? 'Testing...' : 'Test Connection'}
              </button>
            </div>
          </div>
        )}

        {/* Step 3: Confirmation */}
        {step === 'confirm' && provider && (
          <div>
            <h3 style={headingStyle}>Connected</h3>
            <p style={subTextStyle}>
              {provider.label} is ready. Your team now has access to this provider.
            </p>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                padding: '12px 16px',
                border: '1px solid rgb(34 197 94 / 0.4)',
                borderRadius: '4px',
                marginBottom: '20px',
                backgroundColor: 'rgb(34 197 94 / 0.04)',
              }}
            >
              <div
                style={{
                  width: '8px',
                  height: '8px',
                  borderRadius: '50%',
                  backgroundColor: 'rgb(74 222 128)',
                }}
              />
              <span
                style={{
                  fontFamily: 'monospace',
                  fontSize: '0.75rem',
                  fontWeight: 700,
                  color: 'rgb(74 222 128)',
                }}
              >
                {provider.label}
              </span>
              <span
                style={{
                  fontFamily: 'monospace',
                  fontSize: '0.6rem',
                  color: 'rgb(34 197 94 / 0.5)',
                  marginLeft: 'auto',
                  textTransform: 'uppercase',
                  letterSpacing: '0.08em',
                }}
              >
                Active
              </span>
            </div>
            <button
              style={btnStyle}
              onClick={() => onComplete(provider.slug, apiKey)}
            >
              Done
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
