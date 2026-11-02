import { cloneElement, isValidElement, memo, useInsertionEffect, useLayoutEffect, useRef } from 'react';
import type { ReactElement, RefObject } from 'react';

export function sameContent(before: unknown, next: unknown, width = false): boolean {
  if (Object.is(before, next)) return true;
  if (isValidElement<Record<string, unknown>>(before) && isValidElement<Record<string, unknown>>(next))
    return before.type === next.type && before.key === next.key && sameProps(before.props, next.props, typeof before.type === 'string', width);
  if (Array.isArray(before) && Array.isArray(next)) return before.length === next.length && before.every((value, index) => sameContent(value, next[index], width));
  return shallowEqual(before, next);
}

function shallowEqual(before: unknown, next: unknown): boolean {
  if (Object.is(before, next)) return true;
  if (!before || !next || typeof before !== 'object' || typeof next !== 'object'
    || Object.getPrototypeOf(before) !== Object.prototype || Object.getPrototypeOf(next) !== Object.prototype) return false;
  const a = before as Record<string, unknown>, b = next as Record<string, unknown>, keys = Object.keys(a);
  return keys.length === Object.keys(b).length && keys.every(key => Object.hasOwn(b, key) && Object.is(a[key], b[key]));
}

function sameProps(before: Record<string, unknown>, next: Record<string, unknown>, native: boolean, width: boolean): boolean {
  const keys = Object.keys(before);
  return keys.length === Object.keys(next).length && keys.every(key => {
    if (!Object.hasOwn(next, key)) return false;
    if (key === 'children') return sameContent(before[key], next[key], width);
    if ((native || width) && key === 'style') return shallowEqual(before[key], next[key]);
    // Component callback identity can signal a new viewport consumer.
    return (native && isEvent(key, before[key]) && isEvent(key, next[key])) || Object.is(before[key], next[key]);
  });
}

export function useEqualContent<T>(value: T, equal: (before: T, next: T) => boolean = sameContent): T {
  const committed = useRef(value);
  const stable = equal(committed.current, value) ? committed.current : value;
  useLayoutEffect(() => { committed.current = stable; }, [stable]);
  return stable;
}

type RowElement = ReactElement<Record<string, unknown>>;
type RowEvents = RefObject<ReadonlyMap<string, RowElement>>;
const isEvent = (key: string, value: unknown) => /^on[A-Z]/.test(key) && typeof value === 'function';

type Handler = (...args: unknown[]) => unknown;
function bindEvents(value: unknown, events: RowEvents, id: string, handlers: Map<string, Handler>, used: Set<string>, path: readonly (string | number)[] = []): unknown {
  if (Array.isArray(value)) {
    const children = value.map((child, index) => bindEvents(child, events, id, handlers, used, [...path, index]));
    return children.every((child, index) => child === value[index]) ? value : children;
  }
  if (!isValidElement<Record<string, unknown>>(value)) return value;
  const bound: Record<string, unknown> = {};
  for (const key of Object.keys(value.props)) {
    if (typeof value.type === 'string' && isEvent(key, value.props[key])) {
      const at = [...path, 'props', key], token = JSON.stringify(at);
      used.add(token);
      let handler = handlers.get(token);
      if (!handler) {
        handler = (...args) => {
          let latest: unknown = events.current.get(id);
          for (const part of at) latest = (latest as Record<string | number, unknown> | undefined)?.[part];
          return (latest as Handler | undefined)?.(...args);
        };
        handlers.set(token, handler);
      }
      bound[key] = handler;
    } else if (key === 'children') {
      const children = bindEvents(value.props.children, events, id, handlers, used, [...path, 'props', 'children']);
      if (children !== value.props.children) bound.children = children;
    }
  }
  return Object.keys(bound).length ? cloneElement(value, bound) : value;
}

const MemoRow = memo(function MemoRow({ id, element, events }: { id: string; element: RowElement; events: RowEvents }) {
  const handlers = useRef(new Map<string, Handler>());
  const used = new Set<string>();
  const row = bindEvents(element, events, id, handlers.current, used) as RowElement;
  for (const token of handlers.current.keys()) if (!used.has(token)) handlers.current.delete(token);
  return row;
}, (before, next) => before.id === next.id && before.events === next.events && sameContent(before.element, next.element));

export function useMemoRows() {
  const events = useRef<ReadonlyMap<string, RowElement>>(new Map());
  const next = new Map<string, RowElement>();
  // Install handlers before a child's layout effect can focus a nested control.
  useInsertionEffect(() => { events.current = next; });
  return (id: string, element: RowElement) => {
    next.set(id, element);
    return <MemoRow key={id} id={id} element={element} events={events} />;
  };
}
