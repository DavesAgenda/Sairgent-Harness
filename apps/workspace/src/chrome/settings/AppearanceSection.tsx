import { useState } from 'react';

interface AppearanceSectionProps {
  activeSkinId: string;
  availableSkins: { id: string; name: string }[];
  onSkinSelect: (id: string) => void;
}

const ANIMATION_SPEEDS = [
  { id: 'fast', label: 'Fast', value: 0.5 },
  { id: 'normal', label: 'Normal', value: 1.0 },
  { id: 'slow', label: 'Slow', value: 2.0 },
  { id: 'off', label: 'Off', value: 0 },
];

export function AppearanceSection({
  activeSkinId,
  availableSkins,
  onSkinSelect,
}: AppearanceSectionProps) {
  const [animSpeed, setAnimSpeed] = useState('normal');
  const [gridDensity, setGridDensity] = useState<'comfortable' | 'compact'>('comfortable');

  return (
    <div>
      <h2 style={headingStyle}>Appearance</h2>
      <p style={subTextStyle}>Customize how your workspace looks and feels.</p>

      {/* Skin selector */}
      <div style={{ marginBottom: '28px' }}>
        <label style={labelStyle}>Workspace Skin</label>
        <div style={{ display: 'flex', gap: '8px', flexWrap: 'wrap' }}>
          {availableSkins.map((skin) => (
            <button
              key={skin.id}
              onClick={() => onSkinSelect(skin.id)}
              style={{
                fontFamily: 'monospace',
                fontSize: '0.7rem',
                padding: '8px 16px',
                border:
                  activeSkinId === skin.id
                    ? '1px solid rgb(74 222 128)'
                    : '1px solid rgb(34 197 94 / 0.2)',
                borderRadius: '4px',
                backgroundColor:
                  activeSkinId === skin.id
                    ? 'rgb(34 197 94 / 0.08)'
                    : 'transparent',
                color:
                  activeSkinId === skin.id
                    ? 'rgb(74 222 128)'
                    : 'rgb(34 197 94 / 0.5)',
                cursor: 'pointer',
                transition: 'all 0.1s ease',
                textTransform: 'uppercase',
                letterSpacing: '0.08em',
              }}
            >
              {skin.name}
            </button>
          ))}
        </div>
      </div>

      {/* Animation speed */}
      <div style={{ marginBottom: '28px' }}>
        <label style={labelStyle}>Animation Speed</label>
        <div style={{ display: 'flex', gap: '8px' }}>
          {ANIMATION_SPEEDS.map((speed) => (
            <button
              key={speed.id}
              onClick={() => setAnimSpeed(speed.id)}
              style={{
                fontFamily: 'monospace',
                fontSize: '0.65rem',
                padding: '6px 14px',
                border:
                  animSpeed === speed.id
                    ? '1px solid rgb(74 222 128)'
                    : '1px solid rgb(34 197 94 / 0.2)',
                borderRadius: '4px',
                backgroundColor:
                  animSpeed === speed.id
                    ? 'rgb(34 197 94 / 0.08)'
                    : 'transparent',
                color:
                  animSpeed === speed.id
                    ? 'rgb(74 222 128)'
                    : 'rgb(34 197 94 / 0.5)',
                cursor: 'pointer',
                textTransform: 'uppercase',
                letterSpacing: '0.06em',
              }}
            >
              {speed.label}
            </button>
          ))}
        </div>
      </div>

      {/* Grid density */}
      <div style={{ marginBottom: '28px' }}>
        <label style={labelStyle}>Grid Density</label>
        <div style={{ display: 'flex', gap: '8px' }}>
          {(['comfortable', 'compact'] as const).map((density) => (
            <button
              key={density}
              onClick={() => setGridDensity(density)}
              style={{
                fontFamily: 'monospace',
                fontSize: '0.65rem',
                padding: '6px 14px',
                border:
                  gridDensity === density
                    ? '1px solid rgb(74 222 128)'
                    : '1px solid rgb(34 197 94 / 0.2)',
                borderRadius: '4px',
                backgroundColor:
                  gridDensity === density
                    ? 'rgb(34 197 94 / 0.08)'
                    : 'transparent',
                color:
                  gridDensity === density
                    ? 'rgb(74 222 128)'
                    : 'rgb(34 197 94 / 0.5)',
                cursor: 'pointer',
                textTransform: 'uppercase',
                letterSpacing: '0.06em',
              }}
            >
              {density}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

const headingStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: 'var(--ws-font-lg)',
  fontWeight: 700,
  color: 'var(--ws-fg-primary)',
  letterSpacing: '0.1em',
  textTransform: 'uppercase',
  marginBottom: 'var(--ws-space-sm)',
};

const subTextStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: 'var(--ws-font-sm)',
  color: 'var(--ws-fg-secondary)',
  marginBottom: 'var(--ws-space-xl)',
  lineHeight: 1.5,
};

const labelStyle: React.CSSProperties = {
  display: 'block',
  fontFamily: 'monospace',
  fontSize: 'var(--ws-font-sm)',
  fontWeight: 700,
  color: 'var(--ws-fg-secondary)',
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  marginBottom: 'var(--ws-space-md)',
};
