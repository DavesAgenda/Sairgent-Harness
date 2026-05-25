import { useRef, useEffect, useState, useCallback } from 'react';
import type { TubeState } from '../../../types';

interface Props {
  tubes: TubeState[];
  containerRef: React.RefObject<HTMLElement | null>;
}

interface TubeRenderData {
  tube: TubeState;
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
}

const STATUS_COLOR: Record<TubeState['status'], string> = {
  active:   '#3b82f6',
  blocked:  '#ef4444',
  review:   '#a855f7',
  complete: '#22c55e',
};

const STATUS_EMOJI: Record<TubeState['status'], string> = {
  active:   '\uD83D\uDCE6', // package
  blocked:  '\uD83D\uDEA7', // construction
  review:   '\uD83D\uDC40', // eyes
  complete: '\u2705',        // check mark
};

/**
 * Simplified tube overlay for the emoji skin.
 * Uses CSS dashed borders between emoji endpoints instead of SVG paths.
 */
export function EmojiTubeOverlay({ tubes, containerRef }: Props) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [renderData, setRenderData] = useState<TubeRenderData[]>([]);

  const recompute = useCallback(() => {
    const container = containerRef.current;
    if (!container) return;

    const containerRect = container.getBoundingClientRect();
    setSize({ width: containerRect.width, height: containerRect.height });

    const data: TubeRenderData[] = [];
    for (const tube of tubes) {
      const fromEl = container.querySelector<HTMLElement>(`[data-agent-id="${tube.fromAgentId}"]`);
      const toEl   = container.querySelector<HTMLElement>(`[data-agent-id="${tube.toAgentId}"]`);
      if (!fromEl || !toEl) continue;

      const fromRect = fromEl.getBoundingClientRect();
      const toRect   = toEl.getBoundingClientRect();

      data.push({
        tube,
        fromX: fromRect.left + fromRect.width / 2 - containerRect.left,
        fromY: fromRect.bottom - containerRect.top,
        toX: toRect.left + toRect.width / 2 - containerRect.left,
        toY: toRect.top - containerRect.top,
      });
    }
    setRenderData(data);
  }, [tubes, containerRef]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    recompute();
    const ro = new ResizeObserver(() => recompute());
    ro.observe(container);
    return () => ro.disconnect();
  }, [containerRef, recompute]);

  useEffect(() => { recompute(); }, [tubes, recompute]);

  if (size.width === 0 || renderData.length === 0) return null;

  return (
    <svg
      ref={svgRef}
      className="absolute inset-0 pointer-events-none overflow-visible"
      width={size.width}
      height={size.height}
      style={{ zIndex: 10 }}
      aria-hidden="true"
    >
      {renderData.map(({ tube, fromX, fromY, toX, toY }) => {
        const color = STATUS_COLOR[tube.status];
        const emoji = STATUS_EMOJI[tube.status];
        // Capsule position along the line
        const cx = fromX + (toX - fromX) * tube.capsuleProgress;
        const cy = fromY + (toY - fromY) * tube.capsuleProgress;

        return (
          <g key={tube.id}>
            {/* Dashed line */}
            <line
              x1={fromX}
              y1={fromY}
              x2={toX}
              y2={toY}
              stroke={color}
              strokeWidth={2}
              strokeDasharray="8 6"
              strokeOpacity={0.5}
              strokeLinecap="round"
            />
            {/* Emoji capsule */}
            {tube.capsuleProgress > 0 && tube.capsuleProgress < 1 && (
              <text
                x={cx}
                y={cy}
                textAnchor="middle"
                dominantBaseline="central"
                fontSize={16}
              >
                {emoji}
              </text>
            )}
          </g>
        );
      })}
    </svg>
  );
}
