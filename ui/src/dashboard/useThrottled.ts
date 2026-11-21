import { useEffect, useRef, useState } from 'react';
import { inFrame } from './frameUpdates.ts';
import { busyWait } from '../shell/busy.ts';

export function useThrottled<T>(value: T, ms: number): T {
  const [shown, setShown] = useState(value);
  const last = useRef(0);
  const latest = useRef(value); latest.current = value;
  useEffect(() => {
    if (Object.is(value, shown)) return;
    const wait = last.current + ms - Date.now();
    const show = () => { last.current = Date.now(); inFrame(() => setShown(latest.current)); };
    if (wait <= 0) { show(); return; }
    return busyWait(show, wait);
  }, [value, shown, ms]);
  return shown;
}
