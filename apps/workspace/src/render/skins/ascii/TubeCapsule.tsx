import { useEffect } from 'react';
import { motion, useMotionValue } from 'motion/react';
import type { TubeState } from '../../../types';

interface TubeCapsuleProps {
  pathData: string;
  progress: number;
  status: TubeState['status'];
}

const STATUS_COLOR: Record<TubeState['status'], string> = {
  active:   '#3b82f6',
  blocked:  '#ef4444',
  review:   '#a855f7',
  complete: '#22c55e',
};

const STATUS_GLOW: Record<TubeState['status'], string> = {
  active:   '0 0 6px #3b82f6, 0 0 12px #3b82f660',
  blocked:  '0 0 6px #ef4444, 0 0 12px #ef444460',
  review:   '0 0 6px #a855f7, 0 0 12px #a855f760',
  complete: '0 0 6px #22c55e, 0 0 12px #22c55e60',
};

/** More intense glow for active delegation capsules. */
const ACTIVE_GLOW_INTENSE = '0 0 10px #3b82f6, 0 0 20px #3b82f680, 0 0 30px #3b82f640';

function getPointOnPath(pathData: string, progress: number): { x: number; y: number } | null {
  if (typeof document === 'undefined') return null;
  try {
    const svgNS = 'http://www.w3.org/2000/svg';
    const tmpPath = document.createElementNS(svgNS, 'path');
    tmpPath.setAttribute('d', pathData);
    const totalLength = tmpPath.getTotalLength();
    const pt = tmpPath.getPointAtLength(totalLength * Math.max(0, Math.min(1, progress)));
    return { x: pt.x, y: pt.y };
  } catch {
    return null;
  }
}

export function TubeCapsule({ pathData, progress, status }: TubeCapsuleProps) {
  const x = useMotionValue(0);
  const y = useMotionValue(0);

  // Recompute position whenever pathData or progress changes
  useEffect(() => {
    const pt = getPointOnPath(pathData, progress);
    if (pt) {
      x.set(pt.x);
      y.set(pt.y);
    }
  }, [pathData, progress, x, y]);

  // Hide capsule at exact endpoints
  if (progress <= 0 || progress >= 1) return null;

  const color = STATUS_COLOR[status];
  const glow  = status === 'active' ? ACTIVE_GLOW_INTENSE : STATUS_GLOW[status];
  const isActive = status === 'active';

  return (
    <motion.g
      style={{ x, y }}
      initial={{ opacity: 0, scale: 0.5 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.5 }}
      transition={{ duration: 0.2 }}
    >
      {/* Glow halo -- pulses more intensely during active delegation */}
      <motion.ellipse
        cx={0}
        cy={0}
        rx={isActive ? 12 : 8}
        ry={isActive ? 7 : 5}
        fill={color}
        opacity={0.25}
        animate={
          isActive
            ? { opacity: [0.1, 0.5, 0.1], rx: [10, 14, 10], ry: [6, 8, 6] }
            : { opacity: [0.15, 0.35, 0.15] }
        }
        transition={
          isActive
            ? { duration: 0.8, repeat: Infinity, ease: 'easeInOut' }
            : { duration: 1.2, repeat: Infinity, ease: 'easeInOut' }
        }
      />
      {/* Capsule body */}
      <rect
        x={-6}
        y={-3.5}
        width={12}
        height={7}
        rx={3.5}
        fill={color}
        style={{ filter: `drop-shadow(${glow})` }}
      />
      {/* Inner highlight */}
      <rect
        x={-4}
        y={-2}
        width={5}
        height={2}
        rx={1}
        fill="white"
        opacity={0.35}
      />
    </motion.g>
  );
}
