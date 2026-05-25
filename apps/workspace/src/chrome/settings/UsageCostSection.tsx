import { useEffect, useState } from 'react';
import type { AgentTokenTotals } from '../../types';
import type { TauriBus } from '../../sim/tauriBus';

interface UsageCostSectionProps {
  bus?: TauriBus;
}

function formatTokenCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function formatCost(v: number | null): string {
  if (v === null) return '—';
  return `$${v.toFixed(4)}`;
}

export function UsageCostSection({ bus }: UsageCostSectionProps) {
  const [totals, setTotals] = useState<AgentTokenTotals[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!bus) {
      setTotals([]);
      return;
    }
    bus.loadTokenUsageTotals().then(setTotals).catch((e: unknown) => {
      setError(e instanceof Error ? e.message : String(e));
      setTotals([]);
    });
  }, [bus]);

  const grandInput = totals?.reduce((s, t) => s + t.inputTokens, 0) ?? 0;
  const grandOutput = totals?.reduce((s, t) => s + t.outputTokens, 0) ?? 0;
  const grandTotal = totals?.reduce((s, t) => s + t.totalTokens, 0) ?? 0;
  const grandCost = totals?.reduce((s, t) => s + (t.estimatedCostUsd ?? 0), 0) ?? 0;
  const hasCost = totals?.some((t) => t.estimatedCostUsd !== null) ?? false;

  const cellStyle: React.CSSProperties = {
    padding: '6px 10px',
    fontSize: 'var(--ws-font-xs)',
    borderBottom: '1px solid var(--ws-border-subtle)',
    fontVariantNumeric: 'tabular-nums',
    whiteSpace: 'nowrap',
  };

  const headerCellStyle: React.CSSProperties = {
    ...cellStyle,
    color: 'var(--ws-fg-muted)',
    letterSpacing: '0.1em',
    textTransform: 'uppercase',
    fontSize: '0.6rem',
    borderBottom: '1px solid var(--ws-border)',
    fontWeight: 700,
  };

  return (
    <div style={{ fontFamily: 'monospace' }}>
      {/* Section title */}
      <div
        style={{
          fontSize: 'var(--ws-font-base)',
          fontWeight: 700,
          color: 'var(--ws-fg-primary)',
          letterSpacing: '0.15em',
          textTransform: 'uppercase',
          marginBottom: '4px',
        }}
      >
        Usage &amp; Cost
      </div>
      <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', marginBottom: '24px' }}>
        Token consumption across all agents and runs.
      </div>

      {error && (
        <div
          style={{
            padding: '10px 14px',
            backgroundColor: 'rgb(248 113 113 / 0.08)',
            border: '1px solid rgb(248 113 113 / 0.3)',
            borderRadius: 'var(--ws-radius-sm)',
            color: 'rgb(248 113 113)',
            fontSize: 'var(--ws-font-xs)',
            marginBottom: '16px',
          }}
        >
          Failed to load usage data: {error}
        </div>
      )}

      {totals === null ? (
        <div style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-xs)' }}>Loading…</div>
      ) : totals.length === 0 && !error ? (
        <div style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-xs)' }}>
          No usage data recorded yet.
        </div>
      ) : (
        <>
          {/* Grand totals summary row */}
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(4, 1fr)',
              gap: '1px',
              marginBottom: '24px',
              border: '1px solid var(--ws-border)',
              borderRadius: 'var(--ws-radius-sm)',
              overflow: 'hidden',
            }}
          >
            {[
              { label: 'TOTAL INPUT', value: formatTokenCount(grandInput), accent: false },
              { label: 'TOTAL OUTPUT', value: formatTokenCount(grandOutput), accent: false },
              { label: 'TOTAL TOKENS', value: formatTokenCount(grandTotal), accent: false },
              { label: 'EST. COST', value: hasCost ? `$${grandCost.toFixed(4)}` : '—', accent: hasCost },
            ].map(({ label, value, accent }) => (
              <div
                key={label}
                style={{
                  backgroundColor: 'var(--ws-bg-elevated)',
                  padding: '14px 16px',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: '6px',
                }}
              >
                <div
                  style={{
                    fontSize: '0.58rem',
                    color: 'var(--ws-fg-muted)',
                    letterSpacing: '0.12em',
                    textTransform: 'uppercase',
                  }}
                >
                  {label}
                </div>
                <div
                  style={{
                    fontSize: 'var(--ws-font-base)',
                    fontWeight: 700,
                    color: accent ? 'rgb(251 191 36)' : 'var(--ws-fg-primary)',
                    fontVariantNumeric: 'tabular-nums',
                  }}
                >
                  {value}
                </div>
              </div>
            ))}
          </div>

          {/* Per-agent breakdown table */}
          <div
            style={{
              border: '1px solid var(--ws-border)',
              borderRadius: 'var(--ws-radius-sm)',
              overflow: 'hidden',
            }}
          >
            <table
              style={{
                width: '100%',
                borderCollapse: 'collapse',
                fontSize: 'var(--ws-font-xs)',
              }}
            >
              <thead>
                <tr style={{ backgroundColor: 'var(--ws-bg-elevated)' }}>
                  <th style={{ ...headerCellStyle, textAlign: 'left' }}>Agent</th>
                  <th style={{ ...headerCellStyle, textAlign: 'right' }}>Input</th>
                  <th style={{ ...headerCellStyle, textAlign: 'right' }}>Output</th>
                  <th style={{ ...headerCellStyle, textAlign: 'right' }}>Cache Hits</th>
                  <th style={{ ...headerCellStyle, textAlign: 'right' }}>Runs</th>
                  <th style={{ ...headerCellStyle, textAlign: 'right' }}>Est. Cost</th>
                </tr>
              </thead>
              <tbody>
                {totals.map((row) => (
                  <tr
                    key={row.agentId}
                    style={{ backgroundColor: 'transparent' }}
                    onMouseEnter={(e) => {
                      (e.currentTarget as HTMLTableRowElement).style.backgroundColor = 'rgb(255 255 255 / 0.02)';
                    }}
                    onMouseLeave={(e) => {
                      (e.currentTarget as HTMLTableRowElement).style.backgroundColor = 'transparent';
                    }}
                  >
                    <td style={{ ...cellStyle, color: 'rgb(74 222 128)', textAlign: 'left' }}>
                      {row.agentId}
                    </td>
                    <td style={{ ...cellStyle, color: 'var(--ws-fg-primary)', textAlign: 'right' }}>
                      {formatTokenCount(row.inputTokens)}
                    </td>
                    <td style={{ ...cellStyle, color: 'var(--ws-fg-primary)', textAlign: 'right' }}>
                      {formatTokenCount(row.outputTokens)}
                    </td>
                    <td
                      style={{
                        ...cellStyle,
                        color: row.cacheReadTokens > 0 ? 'rgb(74 222 128 / 0.8)' : 'var(--ws-fg-dim)',
                        textAlign: 'right',
                      }}
                    >
                      {formatTokenCount(row.cacheReadTokens)}
                    </td>
                    <td style={{ ...cellStyle, color: 'var(--ws-fg-muted)', textAlign: 'right' }}>
                      {row.runCount}
                    </td>
                    <td
                      style={{
                        ...cellStyle,
                        color: row.estimatedCostUsd !== null ? 'rgb(251 191 36)' : 'var(--ws-fg-dim)',
                        textAlign: 'right',
                        borderBottom: 'none',
                      }}
                    >
                      {formatCost(row.estimatedCostUsd)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </div>
  );
}
