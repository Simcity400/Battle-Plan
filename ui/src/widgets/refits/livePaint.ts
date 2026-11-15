import { useLayoutEffect } from 'react';
import type { RefObject } from 'react';
import type { Live, LiveValues } from '../../charts/live';
import { paintTexts } from '../../dashboard/follow/painters.ts';

/** The texts `texts` makes of the live cells, written into `root`'s `data-paint` elements each time a cell may have
 * moved (every frame of playback, a seek, a push), never by a render; `painted` is told after each paint. */
export function useLivePaint(root: RefObject<Element | null>, live: Live<LiveValues<number>>,
  texts: ((values: LiveValues<number>) => ReadonlyMap<string, string>) | null, painted: () => void): void {
  useLayoutEffect(() => {
    if (!texts) return;
    const paint = () => { paintTexts(root.current, texts(live.read())); painted(); };
    paint();
    return live.subscribe(paint);
  }, [root, live, texts, painted]);
}
