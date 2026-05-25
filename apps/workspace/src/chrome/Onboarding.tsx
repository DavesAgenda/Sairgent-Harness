import { useState, useCallback } from 'react';
import { ConnectionWizard } from './settings/ConnectionWizard';

interface OnboardingProps {
  onComplete: () => void;
}

export function Onboarding({ onComplete }: OnboardingProps) {
  const [wizardOpen, setWizardOpen] = useState(false);

  const handleWizardComplete = useCallback(
    (_provider: string, _key: string) => {
      setWizardOpen(false);
      onComplete();
    },
    [onComplete],
  );

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100vh',
        textAlign: 'center',
        padding: '32px',
      }}
    >
      <div style={{ maxWidth: '440px' }}>
        {/* Title */}
        <h1
          style={{
            fontFamily: 'monospace',
            fontSize: 'var(--ws-font-xl)',
            fontWeight: 700,
            color: 'var(--ws-fg-primary)',
            letterSpacing: '0.15em',
            textTransform: 'uppercase',
            marginBottom: 'var(--ws-space-lg)',
          }}
        >
          Welcome to Sairgent
        </h1>

        {/* Subtitle */}
        <p
          style={{
            fontFamily: 'monospace',
            fontSize: 'var(--ws-font-md)',
            color: 'var(--ws-fg-secondary)',
            lineHeight: 1.6,
            marginBottom: 'var(--ws-space-2xl)',
          }}
        >
          Your AI team is ready. Let's connect them to a brain.
        </p>

        {/* Visual decoration */}
        <div
          style={{
            display: 'flex',
            justifyContent: 'center',
            gap: '12px',
            marginBottom: '40px',
          }}
        >
          {[0, 1, 2, 3, 4].map((i) => (
            <div
              key={i}
              style={{
                width: '8px',
                height: '8px',
                borderRadius: '50%',
                backgroundColor: 'rgb(34 197 94 / 0.2)',
                animation: `pulse 2s ease-in-out ${i * 0.3}s infinite`,
              }}
            />
          ))}
        </div>

        {/* CTA */}
        <button
          onClick={() => setWizardOpen(true)}
          style={{
            fontFamily: 'monospace',
            fontSize: 'var(--ws-font-md)',
            fontWeight: 700,
            letterSpacing: '0.15em',
            textTransform: 'uppercase',
            color: 'var(--ws-bg)',
            backgroundColor: 'var(--ws-fg-primary)',
            border: 'none',
            borderRadius: 'var(--ws-radius-sm)',
            padding: '12px 36px',
            cursor: 'pointer',
            transition: 'all 0.15s ease',
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.backgroundColor = 'var(--ws-accent)';
            e.currentTarget.style.boxShadow = 'var(--ws-shadow-glow)';
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.backgroundColor = 'var(--ws-fg-primary)';
            e.currentTarget.style.boxShadow = 'none';
          }}
        >
          Connect a Provider
        </button>

        {/* Inline keyframe animation */}
        <style>{`
          @keyframes pulse {
            0%, 100% { opacity: 0.3; transform: scale(1); }
            50% { opacity: 1; transform: scale(1.4); }
          }
        `}</style>
      </div>

      {wizardOpen && (
        <ConnectionWizard
          onComplete={handleWizardComplete}
          onCancel={() => setWizardOpen(false)}
        />
      )}
    </div>
  );
}
