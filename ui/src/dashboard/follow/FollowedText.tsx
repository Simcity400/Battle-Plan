import { useLayoutEffect, useRef } from 'react';
import type { FollowEntry } from './store.ts';
import { textCell } from './painters.ts';
import type { TextCell } from './painters.ts';

export function FollowedText({ entry, write }: { entry: FollowEntry; write: (entry: FollowEntry, cell: TextCell) => void }) {
  const span = useRef<HTMLSpanElement>(null);
  const writing = useRef(write);
  writing.current = write;
  useLayoutEffect(() => {
    if (!span.current) return;
    const cell = textCell(span.current);
    const paint = () => { if (entry.version) writing.current(entry, cell); };
    paint();
    return entry.subscribe(paint);
  }, [entry]);
  return <span ref={span}>…</span>;
}
