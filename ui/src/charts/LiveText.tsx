import { useLayoutEffect, useRef } from 'react';
import type { LiveTexts } from './live.ts';

export { liveTexts } from './live.ts';

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
