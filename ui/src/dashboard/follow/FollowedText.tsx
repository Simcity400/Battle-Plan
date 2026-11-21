import { useLayoutEffect, useRef } from 'react';
import type { FollowEntry, FollowStore } from './store.ts';
import { textCell } from './painters.ts';
import type { TextCell } from './painters.ts';
import { measuredWidget } from '../widgetPerf.ts';

export function FollowedText({ store, entry, write }: { store: FollowStore; entry: FollowEntry; write: (entry: FollowEntry, cell: TextCell) => void }) {
  const span = useRef<HTMLSpanElement>(null);
  const writing = useRef(write);
  writing.current = write;
  const widget = measuredWidget();
  useLayoutEffect(() => {
    if (!span.current || !store.get(entry.id)) return;
    const cell = textCell(span.current);
    const painter = store.painter(entry, held => writing.current(held, cell), true, widget);
    return () => painter.dispose();
  }, [store, entry, widget]);
  return <span ref={span}>…</span>;
}
