import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { CountryFlagUrl } from '../components/CountryFlag';
import { ValueState } from '../components/ValueState';
import { measureTooltipScroll, scrollTooltip } from './tooltip';
import type { TooltipContent, TooltipRow } from './tooltip';
import { tooltipRowOffsets, tooltipRowWindow, tooltipScrollTop } from './tooltipScroll';

export function TooltipRowView({ row, element }: { row: TooltipRow; element?: (node: HTMLDivElement | null) => void }) {
  return <div ref={element} data-detail={row.detail ?? row.countryFlag?.nameFailure?.message} title={row.lookupKey ? `Key: ${row.lookupKey}` : undefined} className={`chart-tooltip-row${row.divider ? ' has-divider' : ''}${row.strong ? ' is-strong' : ''}${row.short ? ' is-short' : ''}${row.indent ? ' is-indented' : ''}`}>
    <span>{row.swatch && <i style={{ background: row.swatch }} aria-hidden="true" />}{row.countryFlag
      ? <CountryFlagUrl className="chart-tooltip-flag" url={row.countryFlag.flag} detail={row.countryFlag.flagDetail} reason={row.countryFlag.flagReason} pending={row.countryFlag.flagPending} />
      : row.icon && <img src={row.icon} alt="" />}{row.label}</span>
    {row.valueState
      ? <b className={row.tone ? `is-${row.tone}` : undefined}><ValueState {...row.valueState} /></b>
      : row.value != null && <b className={row.tone ? `is-${row.tone}` : undefined}>{row.value}</b>}
  </div>;
}

export function ScrollableTooltipRows({ content, owner, top }: { content: TooltipContent; owner: string; top: number }) {
  const rows = useRef<HTMLDivElement>(null);
  const elements = useRef(new Map<number, HTMLDivElement>());
  const cache = useRef({ rows: content.rows, measured: new Map<number, number>() });
  if (cache.current.rows !== content.rows) cache.current = { rows: content.rows, measured: new Map() };
  const [revision, setRevision] = useState(0);
  const [viewport, setViewport] = useState({ height: 0, leading: 0, trailing: 0 });
  const { height, leading, trailing } = viewport;
  const offsets = useMemo(() => tooltipRowOffsets(content.rows, cache.current.measured), [content.rows, revision]);
  const scrollTop = tooltipScrollTop(offsets, top, height, leading, trailing);
  const { first, last } = tooltipRowWindow(offsets, scrollTop, height, leading, trailing);
  useLayoutEffect(() => {
    const element = rows.current!;
    const scroller = element.parentElement!;
    let width = element.clientWidth;
    const measure = () => {
      const next = { height: scroller.clientHeight, leading: element.offsetTop,
        trailing: Math.max(0, scroller.scrollHeight - element.offsetTop - element.offsetHeight) };
      setViewport(value => value.height === next.height && value.leading === next.leading && value.trailing === next.trailing ? value : next);
      if (width !== element.clientWidth) { width = element.clientWidth; cache.current.measured.clear(); setRevision(value => value + 1); }
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(scroller);
    for (const child of scroller.children) observer.observe(child);
    return () => observer.disconnect();
  }, [content]);
  useLayoutEffect(() => {
    const scroller = rows.current!.parentElement!;
    measureTooltipScroll(owner, height);
    scroller.scrollTop = scrollTop;
    if (top !== scrollTop && height > 0) scrollTooltip(owner, scrollTop - top);
    const measure = () => {
      let changed = false;
      for (const [index, row] of elements.current) {
        const pixels = row.offsetHeight + (row.classList.contains('has-divider') ? parseFloat(getComputedStyle(row).marginTop) || 0 : 0);
        if (pixels > 0 && cache.current.measured.get(index) !== pixels) { cache.current.measured.set(index, pixels); changed = true; }
      }
      if (changed) setRevision(value => value + 1);
    };
    measure();
    const observer = new ResizeObserver(measure);
    for (const row of elements.current.values()) observer.observe(row);
    return () => observer.disconnect();
  }, [content.rows, first, last, scrollTop, top, height, owner]);
  return <div ref={rows} className="chart-tooltip-rows" style={{ height: offsets[offsets.length - 1] }}>
    <div style={{ paddingTop: offsets[first], paddingBottom: offsets[offsets.length - 1] - offsets[last] }}>
      {content.rows.slice(first, last).map((row, index) => <TooltipRowView key={first + index} row={row}
        element={node => { if (node) elements.current.set(first + index, node); else elements.current.delete(first + index); }} />)}
    </div>
  </div>;
}
