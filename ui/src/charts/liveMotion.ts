import { createContext, useCallback, useContext, useRef } from 'react';

export const LiveMotion = createContext<(() => number | undefined) | null>(null);

export function useMotionNow(): (given: number | undefined) => number | undefined {
  const live = useContext(LiveMotion);
  const latest = useRef(live);
  latest.current = live;
  return useCallback((given: number | undefined) => latest.current ? latest.current() : given, []);
}
