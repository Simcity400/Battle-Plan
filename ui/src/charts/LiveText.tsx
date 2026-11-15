import { useLayoutEffect, useRef } from 'react';
import type { LiveTexts } from './live.ts';

export { liveTexts, separated } from './live.ts';
export type { LiveTexts, TextClock, TextSource } from './live.ts';

/** A text kept current by the frame, never rendered for: React writes it, each frame paints it when it moved. */
export default function LiveText({ live, at }: { live: LiveTexts; at: string }) {
  const element = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const paint = () => {
      const node = element.current, text = live.text(at);
      if (node && node.textContent !== text) node.textContent = text;
    };
    paint();
    return live.subscribe(paint);
  }, [live, at]);
  return <span ref={element}>{live.text(at)}</span>;
}
