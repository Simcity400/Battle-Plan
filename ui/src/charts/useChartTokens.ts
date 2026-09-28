import { useEffect, useState } from 'react';
import { readTokens, subscribeTokens } from './tokens';
import type { ChartTokens } from './tokens';

/** The chart tokens, kept current as the theme or appearance changes. */
export function useChartTokens(): ChartTokens {
  const [tokens, setTokens] = useState(readTokens);
  useEffect(() => subscribeTokens(setTokens), []);
  return tokens;
}

/** The window's height, which bounds every tooltip (`fitTooltip`); kept current so
 * a tooltip is fitted again when the window is resized. */
export function useViewportHeight(): number {
  const [height, setHeight] = useState(() => globalThis.innerHeight);
  useEffect(() => {
    const update = () => setHeight(globalThis.innerHeight);
    globalThis.addEventListener('resize', update);
    return () => globalThis.removeEventListener('resize', update);
  }, []);
  return height;
}
