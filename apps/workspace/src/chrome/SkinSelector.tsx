import type { SkinEntry } from '../render/skinRegistry';

interface SkinSelectorProps {
  activeSkinId: string;
  skins: SkinEntry[];
  onSelect: (id: string) => void;
}

export function SkinSelector({ activeSkinId, skins, onSelect }: SkinSelectorProps) {
  return (
    <select
      value={activeSkinId}
      onChange={(e) => onSelect(e.target.value)}
      aria-label="Select workspace skin"
      style={{
        fontFamily: 'monospace',
        fontSize: '0.7rem',
        letterSpacing: '0.06em',
        color: 'rgb(74 222 128)',
        backgroundColor: 'rgb(9 9 11)',
        border: '1px solid rgb(34 197 94 / 0.4)',
        padding: '3px 8px',
        cursor: 'pointer',
        textTransform: 'uppercase',
        outline: 'none',
      }}
    >
      {skins.map((s) => (
        <option key={s.id} value={s.id}>
          {s.name}
        </option>
      ))}
    </select>
  );
}
