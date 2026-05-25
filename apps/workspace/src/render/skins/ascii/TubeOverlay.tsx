import { useRef, useEffect, useState, useCallback } from 'react';
import type { TubeState } from '../../../types';
import { TubeCapsule } from './TubeCapsule';

interface TubeOverlayProps {
  tubes: TubeState[];
  /** The containing element that the SVG should size itself to. */
  containerRef: React.RefObject<HTMLElement | null>;
}

interface TubeRenderData {
  tube: TubeState;
  pathData: string;
}

const STROKE_COLOR: Record<TubeState['status'], string> = {
  active:   '#3b82f6',
  blocked:  '#ef4444',
  review:   '#a855f7',
  complete: '#22c55e',
};

const STROKE_DASH: Record<TubeState['status'], string | undefined> = {
  active:   '6 4',
  blocked:  '4 3',
  review:   undefined,
  complete: undefined,
};

function buildStraightPath(
  fromRect: DOMRect,
  toRect: DOMRect,
  containerRect: DOMRect,
): string {
  const x1 = fromRect.left + fromRect.width / 2 - containerRect.left;
  const y1 = fromRect.bottom - containerRect.top;
  const x2 = toRect.left + toRect.width / 2 - containerRect.left;
  const y2 = toRect.top - containerRect.top;

  const dropY = y1 + 20;
  const riseY = y2 - 20;

  if (Math.abs(x1 - x2) < 4) {
    // Straight vertical tube
    return `M ${x1} ${y1} L ${x2} ${y2}`;
  }

  const cornerR = 10;
  const goRight = x2 > x1;
  const cx1 = goRight ? x1 + cornerR : x1 - cornerR;
  const cx2 = goRight ? x2 - cornerR : x2 + cornerR;

  return [
    `M ${x1} ${y1}`,
    `L ${x1} ${dropY}`,
    `Q ${x1} ${dropY + cornerR} ${cx1} ${dropY + cornerR}`,
    `L ${cx2} ${dropY + cornerR}`,
    `Q ${x2} ${dropY + cornerR} ${x2} ${dropY + cornerR * 2}`,
    `L ${x2} ${riseY}`,
    `Q ${x2} ${riseY + cornerR} ${x2} ${y2}`,
  ].join(' ');
}

export function TubeOverlay({ tubes, containerRef }: TubeOverlayProps) {
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

      const pathData = buildStraightPath(fromRect, toRect, containerRect);
      data.push({ tube, pathData });
    }

    setRenderData(data);
  }, [tubes, containerRef]);

  // ResizeObserver on the container
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    recompute();

    const ro = new ResizeObserver(() => recompute());
    ro.observe(container);
    return () => ro.disconnect();
  }, [containerRef, recompute]);

  // Recompute when tubes change
  useEffect(() => {
    recompute();
  }, [tubes, recompute]);

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
      <defs>
        {/* Subtle blur filter for path glow */}
        <filter id="tube-glow" x="-20%" y="-20%" width="140%" height="140%">
          <feGaussianBlur stdDeviation="2" result="blur" />
          <feMerge>
            <feMergeNode in="blur" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>

      {renderData.map(({ tube, pathData }) => {
        const color = STROKE_COLOR[tube.status];
        const dash  = STROKE_DASH[tube.status];

        return (
          <g key={tube.id}>
            {/* Shadow/glow track */}
            <path
              d={pathData}
              fill="none"
              stroke={color}
              strokeWidth={3}
              strokeOpacity={0.15}
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            {/* Main tube path */}
            <path
              d={pathData}
              fill="none"
              stroke={color}
              strokeWidth={1.5}
              strokeOpacity={0.8}
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeDasharray={dash}
              filter="url(#tube-glow)"
            />
            {/* Capsule */}
            <TubeCapsule
              pathData={pathData}
              progress={tube.capsuleProgress}
              status={tube.status}
            />
          </g>
        );
      })}
    </svg>
  );
}
