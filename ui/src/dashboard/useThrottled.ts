import { useEffect, useRef, useState } from 'react';
import { inFrame } from './frameUpdates.ts';

/** `value`, changing at most once every `ms` milliseconds: a change after a quiet
 * interval shows at once, later ones wait for the interval's end, and the latest
 * value always shows in the end. For reading facts about a moving playhead (names
 * and flags at the hour) without a request for every hour played. */
export function useThrottled<T>(value: T, ms: number): T {
  const [shown, setShown] = useState(value);
  const last = useRef(0);
  const latest = useRef(value); latest.current = value;
  useEffect(() => {
    if (Object.is(value, shown)) return;
    const wait = last.current + ms - Date.now();
    // Rendered with the clock's next tick while playing, as the latest value by then.
    const show = () => { last.current = Date.now(); inFrame(() => setShown(latest.current)); };
    if (wait <= 0) { show(); return; }
    const timer = setTimeout(show, wait);
    return () => clearTimeout(timer);
  }, [value, shown, ms]);
  return shown;
}
