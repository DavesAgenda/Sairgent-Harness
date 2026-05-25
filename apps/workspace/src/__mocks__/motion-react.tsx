/**
 * Vitest mock for motion/react.
 * Replaces animated components with plain HTML equivalents to avoid
 * the React version conflict between the local workspace (React 18)
 * and the monorepo root (React 19) that framer-motion depends on.
 */
import React from 'react';

type MotionProps = React.HTMLAttributes<HTMLElement> & {
  animate?: unknown;
  initial?: unknown;
  exit?: unknown;
  transition?: unknown;
  whileHover?: unknown;
  whileTap?: unknown;
  layout?: unknown;
  layoutId?: unknown;
  variants?: unknown;
  style?: React.CSSProperties;
};

function createMotionComponent(tag: string) {
  return React.forwardRef<HTMLElement, MotionProps>(function MotionComponent(
    { animate, initial, exit, transition, whileHover, whileTap, layout, layoutId, variants, ...rest },
    ref,
  ) {
    return React.createElement(tag, { ...rest, ref });
  });
}

export const motion = new Proxy(
  {},
  {
    get(_target, prop: string) {
      return createMotionComponent(prop);
    },
  },
) as Record<string, ReturnType<typeof createMotionComponent>>;

export function AnimatePresence({ children }: { children?: React.ReactNode }) {
  return React.createElement(React.Fragment, null, children);
}

export function useMotionValue(initial: number) {
  const ref = React.useRef(initial);
  return {
    get: () => ref.current,
    set: (v: number) => { ref.current = v; },
    on: () => () => {},
  };
}

export function useTransform(_value: unknown, _input: unknown, _output: unknown) {
  return useMotionValue(0);
}

export function useSpring(_value: unknown) {
  return useMotionValue(0);
}

export function useScroll() {
  return { scrollY: useMotionValue(0), scrollYProgress: useMotionValue(0) };
}
