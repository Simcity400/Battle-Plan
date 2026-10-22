import { useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { moveIndex, revealRow, visibleRange } from '../components/countryList';
import { movingRowClass, useRowMotion } from './useRowMotion';
import type { MovingRow } from './rowMotion';
import { useParityRows } from '../dashboard/parityRows';

/** An unchanged viewport keeps its state, so measuring and scrolling never re-render for nothing. */
const viewAt = (top: number, height: number) => (before: { top: number; height: number }) =>
  before.top === top && before.height === height ? before : { top, height };

/** Moving rows with bounded DOM and keyboard access to every row. Motion retains
 * the full order; only the viewport and overscan render or request artwork. */
export default function WindowedRows<T>({ items, rowHeight: preferredHeight, fit = false, minRowHeight = 20, padding = 0, label, className, onVisibleRows, empty, children }: {
  items: readonly { key: string; item: T }[]; rowHeight: number; fit?: boolean; minRowHeight?: number; padding?: number; label: string; className?: string;
  /** Reports mounted rows, including their old content while they fade out. */
  onVisibleRows?: (rows: readonly MovingRow<T>[]) => void; empty?: ReactNode;
  children: (row: MovingRow<T>) => ReactNode;
}) {
  const root = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 300 });
  const rowHeight = fit ? Math.min(preferredHeight, Math.max(minRowHeight, view.height / Math.max(1, items.length))) : preferredHeight;
  const [focus, setFocus] = useState<{ at: number; backwards: boolean } | null>(null);
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const moving = useRowMotion(items);
  if (import.meta.env.MODE === 'parity') {
  useParityRows(root, () => ({ kind: 'windowed', label, complete: true, total: items.length,
    rows: items.map(({ key, item }, index) => ({ key, index, item })) }));
  }
  const count = moving.reduce((count, row) => Math.max(count, row.slot + 1), items.length);
  const range = visibleRange(count, Math.max(0, view.top - padding), view.height, rowHeight);
  const visible = moving.filter(row => row.slot >= range.start && row.slot < range.end);
  const focused = moving.find(row => row.key === focusedKey && row.phase !== 'leaving');
  if (focused && !visible.includes(focused)) { visible.push(focused); visible.sort((a, b) => a.slot - b.slot); }
  const focusables = (node: HTMLElement) => [...node.querySelectorAll<HTMLElement>('summary, button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]')]
    .filter(target => target.getClientRects().length > 0);
  const reported = useRef<{ rows: readonly MovingRow<T>[]; callback: typeof onVisibleRows } | null>(null);
  useLayoutEffect(() => {
    const previous = reported.current?.rows;
    if (reported.current?.callback === onVisibleRows && previous && previous.length === visible.length && previous.every((row, index) => row.key === visible[index].key
      && row.item === visible[index].item && row.slot === visible[index].slot && row.phase === visible[index].phase)) return;
    reported.current = { rows: visible, callback: onVisibleRows };
    onVisibleRows?.(visible);
  }, [visible, onVisibleRows]);
  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const measure = () => setView(viewAt(element.scrollTop, element.clientHeight));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    if (focus == null) return;
    const row = root.current?.querySelector<HTMLElement>(`[data-row-index="${focus.at}"]:not([aria-hidden])`);
    const targets = row ? focusables(row) : [];
    const target = focus.backwards ? targets.at(-1) : targets[0];
    if (target) { target.focus(); setFocus(null); }
  }, [focus, range.start, range.end]);
  // Native Tab enters mounted edge rows, not necessarily the list's logical first or last row.
  const tabbing = useRef<boolean | null>(null);
  useLayoutEffect(() => {
    const document = root.current?.ownerDocument;
    if (!document) return;
    let timer = 0;
    const keyboard = (event: KeyboardEvent) => {
      clearTimeout(timer);
      tabbing.current = event.key === 'Tab' ? event.shiftKey : null;
      if (tabbing.current !== null) timer = window.setTimeout(() => { tabbing.current = null; }, 0);
    };
    document.addEventListener('keydown', keyboard, true);
    return () => { clearTimeout(timer); document.removeEventListener('keydown', keyboard, true); };
  }, []);
  const seek = (at: number, backwards: boolean) => {
    if (!root.current) return;
    root.current.scrollTop = padding + revealRow(at, Math.max(0, root.current.scrollTop - padding), view.height, rowHeight);
    setView(viewAt(root.current.scrollTop, root.current.clientHeight));
    setFocus({ at, backwards });
  };
  return <div ref={root} className={`windowed-rows ${className ?? ''}`} role="list" aria-label={label}
    onFocus={event => {
      const backwards = tabbing.current;
      if (backwards === null || event.currentTarget.contains(event.relatedTarget) || !items.length) return;
      const at = backwards ? items.length - 1 : 0;
      const row = (event.target as HTMLElement).closest<HTMLElement>('[data-row-index]');
      if (row && Number(row.dataset.rowIndex) === at) return;
      seek(at, backwards);
    }}
    onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget)) setFocusedKey(null); }}
    onScroll={event => { setView(viewAt(event.currentTarget.scrollTop, event.currentTarget.clientHeight)); }}>
    {!moving.length && empty}
    <div className="windowed-rows-body" style={{ height: padding * 2 + count * rowHeight }}>
      {visible.map(row => <div key={row.key}
        className={movingRowClass(row)} style={{ height: rowHeight, transform: `translateY(${padding + row.slot * rowHeight}px)` }}
        role="listitem" aria-hidden={row.phase === 'leaving' || undefined}
        aria-setsize={items.length} aria-posinset={row.slot + 1} data-row-index={row.slot}
        onFocusCapture={() => setFocusedKey(row.key)}
        onKeyDown={event => {
          if (event.defaultPrevented) return;
          const tab = event.key === 'Tab';
          if (tab) {
            const targets = focusables(event.currentTarget);
            if (event.target !== (event.shiftKey ? targets[0] : targets.at(-1))) return;
          } else if (event.target instanceof HTMLInputElement) return;
          const next = tab ? row.slot + (event.shiftKey ? -1 : 1) : moveIndex(event.key, row.slot, items.length);
          if (next == null || !root.current) return;
          if (next < 0 || next >= items.length) return;
          event.preventDefault();
          seek(next, tab && event.shiftKey);
        }}>{children(row)}</div>)}
    </div>
  </div>;
}
