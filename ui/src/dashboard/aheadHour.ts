import { useRef } from 'react';
import { useHeadHour } from '../widgets/army-size/headHour.ts';
import type { HeadClock } from '../widgets/army-size/headHour';

export interface AheadBlock { hour: number; until: number }

export function aheadKey(held: AheadBlock | null, hour: number): string {
  return `${held && hour >= held.hour && hour < held.until ? held.hour : hour}`;
}

export function useAheadHour(clock: HeadClock, enabled: boolean, end: (hour: number) => number): AheadBlock {
  const held = useRef<AheadBlock | null>(null);
  const hour = useHeadHour(clock, enabled, at => aheadKey(held.current, at));
  const memo = useRef<{ hour: number; end: (hour: number) => number; playing: boolean; block: AheadBlock } | null>(null);
  if (!enabled) { held.current = null; return { hour, until: hour + 1 }; }
  const playing = clock.getSnapshot().playing;
  const before = memo.current;
  const block = before && before.hour === hour && before.end === end && before.playing === playing ? before.block : { hour, until: end(hour) };
  memo.current = { hour, end, playing, block };
  held.current = block;
  return block;
}
