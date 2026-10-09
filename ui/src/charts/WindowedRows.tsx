import { useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { moveIndex, revealRow, visibleRange } from '../components/countryList';
import { movingRowClass, useRowMotion } from './useRowMotion';
import type { MovingRow } from './rowMotion';

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
  const [focus, setFocus] = useState<number | null>(null);
  const moving = useRowMotion(items);
  const count = moving.reduce((count, row) => Math.max(count, row.slot + 1), items.length);
  const range = visibleRange(count, Math.max(0, view.top - padding), view.height, rowHeight);
  const visible = moving.filter(row => row.slot >= range.start && row.slot < range.end);
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
    const measure = () => setView({ top: element.scrollTop, height: element.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    if (focus == null) return;
    const target = root.current?.querySelector<HTMLElement>(`[data-row-index="${focus}"] [tabindex="0"]`);
    if (target) { target.focus(); setFocus(null); }
  }, [focus, range.start, range.end]);
  return <div ref={root} className={`windowed-rows ${className ?? ''}`} role="list" aria-label={label}
    onScroll={event => { setView({ top: event.currentTarget.scrollTop, height: event.currentTarget.clientHeight }); }}>
    {!moving.length && empty}
    <div className="windowed-rows-body" style={{ height: padding * 2 + count * rowHeight }}>
      {visible.map(row => <div key={row.key}
        className={movingRowClass(row)} style={{ height: rowHeight, transform: `translateY(${padding + row.slot * rowHeight}px)` }}
        role="listitem" aria-hidden={row.phase === 'leaving' || undefined}
        aria-setsize={items.length} aria-posinset={row.slot + 1} data-row-index={row.slot}
        onKeyDown={event => {
          const next = moveIndex(event.key, row.slot, items.length);
          if (next == null || !root.current) return;
          event.preventDefault();
          root.current.scrollTop = padding + revealRow(next, Math.max(0, root.current.scrollTop - padding), view.height, rowHeight);
          setView({ top: root.current.scrollTop, height: root.current.clientHeight });
          setFocus(next);
        }}>{children(row)}</div>)}
    </div>
  </div>;
}
