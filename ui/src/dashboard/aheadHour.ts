import { useRef } from 'react';
import { useHeadHour } from '../widgets/army-size/headHour.ts';
import type { HeadClock } from '../widgets/army-size/headHour';

/** A block of hours a list renders once: from `hour` (the clock's hour it rendered at) to before `until`. */
export interface AheadBlock { hour: number; until: number }

/** The key a block's hours share: the clock's hour inside the held block is the block's, elsewhere a block of its own. */
export function aheadKey(held: AheadBlock | null, hour: number): string {
  return `${held && hour >= held.hour && hour < held.until ? held.hour : hour}`;
}

/** Following, the block of hours a list renders its rows for: rendered at the hour the clock entered it, with the rows
 * revealed before `until` drawn ahead for the frame loop to reveal (`LogList` and `Table` take a `RevealClock`). It
 * renders again only when the clock leaves the block, never for a row revealed inside it.
 * `end(hour)` is the end of a block rendered at `hour` (`aheadEnd`); the same function while the rows stand. Not
 * enabled, the block is the clock's hour as rendered. */
export function useAheadHour(clock: HeadClock, enabled: boolean, end: (hour: number) => number): AheadBlock {
  const held = useRef<AheadBlock | null>(null);
  const hour = useHeadHour(clock, enabled, at => aheadKey(held.current, at));
  const memo = useRef<{ hour: number; end: (hour: number) => number; playing: boolean; block: AheadBlock } | null>(null);
  if (!enabled) { held.current = null; return { hour, until: hour + 1 }; }
  const playing = clock.getSnapshot().playing;
  const before = memo.current;
  // A block keeps its end while its rows and the clock's play state stand: a render inside it for another reason
  // (a page landing) must not move the hour it renders again at.
  const block = before && before.hour === hour && before.end === end && before.playing === playing ? before.block : { hour, until: end(hour) };
  memo.current = { hour, end, playing, block };
  held.current = block;
  return block;
}
