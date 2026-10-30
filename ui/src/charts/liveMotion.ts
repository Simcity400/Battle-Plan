import { createContext, useCallback, useContext, useRef } from 'react';

/** A dashboard's playback rate now (its clock's `getMotion`), which charts read as they draw. */
export const LiveMotion = createContext<(() => number | undefined) | null>(null);

/** The rate to ease a frame at: a dashboard's, read as the frame is drawn, so a chart's own
 * change (a zoom, a drill) eases only while playback runs whenever its widget last rendered;
 * outside a dashboard, the rate `given` by its props. */
export function useMotionNow(): (given: number | undefined) => number | undefined {
  const live = useContext(LiveMotion);
  const latest = useRef(live);
  latest.current = live;
  return useCallback((given: number | undefined) => latest.current ? latest.current() : given, []);
}
