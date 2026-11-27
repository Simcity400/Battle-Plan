import { useEffect, useState } from 'react';
import { readTokens, subscribeTokens } from './tokens';
import type { ChartTokens } from './tokens';

export function useChartTokens(): ChartTokens {
  const [tokens, setTokens] = useState(readTokens);
  useEffect(() => subscribeTokens(setTokens), []);
  return tokens;
}
