import { useLayoutEffect, useRef } from 'react';
import { setText } from '../../charts/domText';
import type { Live, LiveValues } from '../../charts/live';

export function LiveText({ live, cell, format, onPaint }: {
  live: Live<LiveValues<number>>; cell: string; format: (value: number | undefined, missing: string) => string; onPaint?: () => void;
}) {
  const span = useRef<HTMLSpanElement>(null);
  const formatting = useRef(format);
  formatting.current = format;
  const painted = useRef(onPaint);
  painted.current = onPaint;
  useLayoutEffect(() => {
    const write = () => {
      const values = live.read();
      setText(span.current, formatting.current(values.get(cell), values.missing(cell)));
      painted.current?.();
    };
    write();
    return live.subscribe(write);
  }, [live, cell]);
  return <span ref={span} />;
}
