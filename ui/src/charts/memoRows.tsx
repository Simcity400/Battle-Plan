import { memo, useInsertionEffect, useLayoutEffect, useMemo, useRef } from 'react';
import type { HTMLAttributes, ReactNode, RefObject } from 'react';

export type RowRevision = readonly unknown[];
export const sameRevision = (before: RowRevision, next: RowRevision): boolean => before.length === next.length
  && before.every((value, index) => Object.is(value, next[index]));

export function useRevision<T extends RowRevision>(value: T): T {
  const committed = useRef(value);
  const stable = sameRevision(committed.current, value) ? committed.current : value;
  useLayoutEffect(() => { committed.current = stable; }, [stable]);
  return stable;
}

export type RowEvents = Pick<HTMLAttributes<HTMLTableRowElement>, 'onPointerMove' | 'onFocus' | 'onBlur' | 'onClick' | 'onContextMenu' | 'onKeyDown' | 'onPointerDown'>;
type Events = RefObject<ReadonlyMap<string, RowEvents>>;
const MemoRow = memo(function MemoRow({ id, render, events }: {
  id: string; revision: RowRevision; render: (handlers: RowEvents) => ReactNode; events: Events;
}) {
  const handlers = useMemo<RowEvents>(() => ({
    onPointerMove: event => events.current.get(id)?.onPointerMove?.(event),
    onFocus: event => events.current.get(id)?.onFocus?.(event),
    onBlur: event => events.current.get(id)?.onBlur?.(event),
    onClick: event => events.current.get(id)?.onClick?.(event),
    onContextMenu: event => events.current.get(id)?.onContextMenu?.(event),
    onKeyDown: event => events.current.get(id)?.onKeyDown?.(event),
    onPointerDown: event => events.current.get(id)?.onPointerDown?.(event),
  }), [id, events]);
  return render(handlers);
}, (before, next) => before.id === next.id && before.events === next.events && sameRevision(before.revision, next.revision));

export function useMemoRows() {
  const events = useRef<ReadonlyMap<string, RowEvents>>(new Map());
  const next = new Map<string, RowEvents>();
  // Nested controls may focus in a layout effect before the parent commits its effects.
  useInsertionEffect(() => { events.current = next; });
  return (id: string, revision: RowRevision, render: (handlers: RowEvents) => ReactNode, handlers: RowEvents = {}) => {
    next.set(id, handlers);
    return <MemoRow key={id} id={id} revision={revision} render={render} events={events} />;
  };
}
