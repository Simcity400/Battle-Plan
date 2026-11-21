import { useEffect, useState } from 'react';
import { readTokens, subscribeTokens } from './tokens';
import type { ChartTokens } from './tokens';

export function useChartTokens(): ChartTokens {
  const [tokens, setTokens] = useState(readTokens);
  useEffect(() => subscribeTokens(setTokens), []);
  return tokens;
}

export function useViewportHeight(): number {
  const [height, setHeight] = useState(() => globalThis.innerHeight);
  useEffect(() => {
    const update = () => setHeight(globalThis.innerHeight);
    globalThis.addEventListener('resize', update);
    return () => globalThis.removeEventListener('resize', update);
  }, []);
  return height;
}
