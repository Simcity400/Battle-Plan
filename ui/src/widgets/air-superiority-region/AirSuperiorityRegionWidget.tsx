import { useCallback, useLayoutEffect, useMemo, useRef } from 'react';
import type { CSSProperties } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import Legend from '../../charts/Legend';
import WindowedRows from '../../charts/WindowedRows';
import { hatchStyle } from '../../charts/hatch';
import { jumpItem } from '../../charts/jump';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { fitTooltip, showTooltip, tooltipText } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { easeTau } from '../../charts/easing';
import { mix, parseColor, readableText } from '../../shell/appearance/color';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useShownHour, useWhyCommit } from '../../dashboard/widgetPerf';
import { useDistinctState } from '../../dashboard/useDistinctState';
import { airPercent, airState, airSummaryTooltip, airTooltip, momentViews, orderedRegions } from '../../dashboard/airSuperiority';
import type { AirRegionView, AirView } from '../../dashboard/airSuperiority';
import { useAirChoicesReply, useAirMoment } from '../../dashboard/useAirSuperiority';
import { OPTIONS } from './definition';
import './airSuperiorityRegion.css';

const LABEL_MIN = 34, SEAM_MIN = 3;

type Order = 'name' | 'active' | 'contested';

const rowsKey = (rows: readonly AirRegionView[]) => rows.map(row => row.region.id).join(',');
const sameAdopted = (held: { source: unknown; key: string }, next: { source: unknown; key: string }) => held.source === next.source && held.key === next.key;

export default function AirSuperiorityRegionWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const order: Order = options.order === 'name' ? 'name' : options.order === 'active' ? 'active' : 'contested';
  const air = useAirMoment(props, options);
  const choices = useAirChoicesReply(runtime, widget, active && order === 'name');
  const menu = useContextMenu('Air superiority region');
  const shown = useShownHour();
  const viewport = useViewportHeight();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const { data, live, looks, tokens } = air;
  const chosenRegions = air.mode.regions;
  const choiceRegions = useMemo(() => order === 'name' ? choices.data?.regions.map(region => ({ id: Number(region.id), name: region.name }))
    .filter(region => !chosenRegions.length || chosenRegions.includes(region.id)) : undefined, [order, choices.data, chosenRegions]);
  const current = air.current;
  const viewsAt = useCallback(() => { const now = current(); return now ? momentViews(now, live.read(), looks, choiceRegions) : []; }, [current, live, looks, choiceRegions]);
  const [adopted, setAdopted] = useDistinctState<{ source: unknown; key: string }>({ source: null, key: '' }, sameAdopted);
  useWhyCommit('AirSuperiorityRegionWidget', { data, looks, adopted, choiceRegions, read: air.read, at: air.at });
  const source = useMemo(() => [data, looks, choiceRegions, order], [data, looks, choiceRegions, order]);
  const fresh = adopted.source !== source ? rowsKey(orderedRegions(viewsAt(), order)) : adopted.key;
  const rowIds = useMemo(() => fresh ? fresh.split(',').map(Number) : [], [fresh]);
  const regionsById = useMemo(() => new Map((data?.regions ?? []).map(region => [region.id, region])), [data]);
  const items = useMemo(() => rowIds.map(id => ({ key: String(id), item: id })), [rowIds]);
  const viewOf = useCallback((id: number) => viewsAt().find(row => row.region.id === id), [viewsAt]);
  const tooltip = useCallback((id: number) => {
    const row = viewOf(id);
    if (!row) return null;
    const full = airTooltip(row.region, row.view, air.mode.mode === 'viewpoint', air.country, !!air.mode.sides.length, air.faction);
    return fitTooltip([airSummaryTooltip(full, row.view)], viewport, { keep: row.view.wings == null ? 1 : 2, full });
  }, [viewOf, air.mode.mode, air.mode.sides.length, air.country, air.faction, viewport]);
  const onMenu = useCallback((_: number, x: number, y: number) => {
    const hour = air.shownAt() ?? air.at;
    if (air.track && hour != null) menu.open(x, y, [jumpItem(runtime, air.track.id, hour)]);
  }, [air.track, air.shownAt, air.at, menu.open, runtime]);
  const owner = `air-regions:${widget.id}`;
  const tips = useRowTooltips({ owner, active, content: tooltip, revision: `${air.at}`, onMenu });
  const pointer = useRef<{ id: number; x: number; y: number } | null>(null);
  const inactive = mix(tokens.surface, tokens.text, 0.1);
  const textOn = useMemo(() => {
    const known = new Map<string, string>();
    return (colour: string) => {
      let text = known.get(colour);
      if (text === undefined) { text = readableText(parseColor(colour) ?? parseColor(tokens.surface)!); known.set(colour, text); }
      return text;
    };
  }, [tokens.surface]);
  const widths = useRef(new Map<number, number>());
  const bars = useRef(new Map<number, HTMLDivElement>());
  const wingsCells = useRef(new Map<number, HTMLSpanElement>());
  const repaint = useRef<(() => void) | null>(null);
  const observer = useMemo(() => typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(entries => {
    let moved = false;
    for (const { target, contentRect } of entries) {
      const id = Number((target as HTMLElement).dataset.region);
      if (widths.current.get(id) !== contentRect.width) { widths.current.set(id, contentRect.width); moved = true; }
    }
    if (moved) repaint.current?.();
  }), []);
  const bar = useCallback((id: number) => (element: HTMLDivElement | null) => {
    const held = bars.current.get(id);
    if (held && held !== element) observer?.unobserve(held);
    if (element) { bars.current.set(id, element); observer?.observe(element); } else bars.current.delete(id);
  }, [observer]);
  const wingsCell = useCallback((id: number) => (element: HTMLSpanElement | null) => {
    if (element) wingsCells.current.set(id, element); else wingsCells.current.delete(id);
  }, []);
  const root = useRef<HTMLDivElement | null>(null);
  const attachRoot = air.root;
  const attach = useCallback((element: HTMLDivElement | null) => { root.current = element; attachRoot(element); }, [attachRoot]);
  const glide = Math.round(3 * easeTau(tokens.motion, hoursPerSecond, tokens.reducedMotion));
  const painted = useRef({ source, rowsKey: fresh, viewsAt, tooltip, inactive, textOn, shownAt: air.shownAt, hatch: hatchStyle(tokens) });
  painted.current = { source, rowsKey: fresh, viewsAt, tooltip, inactive, textOn, shownAt: air.shownAt, hatch: hatchStyle(tokens) };
  useLayoutEffect(() => {
    if (!data) return undefined;
    const paint = () => {
      const now = painted.current;
      const views = now.viewsAt();
      const ordered = orderedRegions(views, order);
      const key = rowsKey(ordered);
      if (key !== now.rowsKey) setAdopted({ source: now.source, key });
      const byId = new Map(views.map(row => [row.region.id, row]));
      for (const [id, element] of bars.current) {
        const row = byId.get(id);
        if (row) paintBar(element, row.view, widths.current.get(id), now.inactive, now.textOn, now.hatch);
      }
      for (const [id, element] of wingsCells.current) {
        const wings = byId.get(id)?.view.wings;
        const text = wings == null ? 'Wings unknown' : `${wings} ${wings === 1 ? 'wing' : 'wings'}`;
        if (element.textContent !== text) element.textContent = text;
      }
      if (root.current) root.current.dataset.glide = String(runtime.clock.getSnapshot().playing);
      const held = pointer.current;
      if (held && active) {
        const content = now.tooltip(held.id);
        if (content) showTooltip(owner, content, held.x, held.y, `${held.id}:${now.shownAt() ?? ''}`);
      }
      shown(now.shownAt());
    };
    paint();
    repaint.current = paint;
    const stops = [live.subscribe(paint), runtime.clock.subscribeFrame(() => { if (root.current) root.current.dataset.glide = String(runtime.clock.getSnapshot().playing); })];
    return () => { for (const stop of stops) stop(); if (repaint.current === paint) repaint.current = null; };
  }, [data, live, order, items, runtime.clock, active, owner, shown, setAdopted]);
  useLayoutEffect(() => () => observer?.disconnect(), [observer]);
  if (air.needsViewpoint) return <WidgetState kind="empty" icon="plane" message="Choose a dashboard country or a viewpoint in the widget settings." />;
  if (!air.track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see its air superiority." />;
  if (air.read === 'off' && !data) return <div ref={attach}><WidgetState kind="outside" /></div>;
  if (air.error) return <div ref={attach}><WidgetState kind="error" failure={air.error} onRetry={air.retry} /></div>;
  if (!data) return <div ref={attach}><WidgetState kind="loading" label="Loading air superiority…" /></div>;
  if (order === 'name' && !choices.data && !choices.error) return <div ref={attach}><WidgetState kind="loading" label="Loading air regions…" /></div>;
  if (data.missing) return <div ref={attach}><WidgetState kind="unavailable" message={data.missing} /></div>;
  const newest = current() ?? data;
  const knownEmpty = newest.fallback.kind !== 'reason' && newest.regions.every(region => region.wings !== 'unknown');
  return <div ref={attach} className="air-regions" data-glide="false" style={{ '--air-glide': `${glide}ms` } as CSSProperties}
    onPointerMove={event => { const row = (event.target as HTMLElement).closest<HTMLElement>('[data-row]'); pointer.current = row ? { id: Number(row.dataset.row), x: event.clientX, y: event.clientY } : null; }}
    onPointerLeave={() => { pointer.current = null; }}>
    <Legend label="Air superiority sides" items={air.legend} always windowed />
    <WindowedRows items={items} rowHeight={38} fit minRowHeight={16} className="air-region-scroll" label="Air superiority by region"
      empty={<WidgetState kind="empty" message={knownEmpty ? 'No regions have a score or assigned wings at this time.' : 'Air activity is unknown at this time.'} />}>
      {({ item: id }) => {
        const region = regionsById.get(id);
        const name = region ? region.name ?? `Region ${id}` : choiceRegions?.find(choice => choice.id === id)?.name ?? `Region ${id}`;
        const description = tips.hovered === id ? tooltip(id) : null;
        return <div className="air-region-row" data-row={id} {...tips.rowProps(id)}>
          {description && <span id={tips.descriptionId(id)} className="sr-only">{tooltipText(description)}</span>}
          <span className="air-region-name" title={region?.nameMissing ?? undefined}>{name}</span>
          <div className="air-region-bar" ref={bar(id)} data-region={id} style={{ backgroundColor: inactive }} />
          <span className="air-region-wings" ref={wingsCell(id)} />
        </div>;
      }}
    </WindowedRows>
    {menu.menu}
  </div>;
}

function paintBar(element: HTMLDivElement, view: AirView, width: number | undefined, inactive: string, textOn: (colour: string) => string, hatch: CSSProperties) {
  const parts = view.state === 'known' ? view.parts : [];
  const signature = view.state === 'known' ? parts.map(part => `${part.id}|${part.color}`).join(';') : `state:${airState(view)}:${view.state}`;
  if (element.dataset.parts !== signature) {
    element.dataset.parts = signature;
    element.replaceChildren();
    element.style.backgroundColor = inactive;
    element.style.backgroundImage = view.state === 'unknown' ? String(hatch.backgroundImage ?? '') : '';
    element.style.backgroundSize = view.state === 'unknown' ? String(hatch.backgroundSize ?? '') : '';
    if (view.state === 'known') {
      for (const part of parts) {
        const segment = document.createElement('span');
        segment.className = 'air-region-segment';
        segment.style.background = part.color;
        segment.style.color = textOn(part.color);
        segment.append(document.createElement('b'));
        const seam = document.createElement('i');
        seam.className = 'air-region-seam';
        seam.setAttribute('aria-hidden', 'true');
        segment.append(seam);
        element.append(segment);
      }
    } else {
      const state = document.createElement('span');
      state.className = 'air-region-state';
      state.textContent = airState(view);
      element.append(state);
    }
  }
  if (view.state !== 'known') return;
  parts.forEach((part, index) => {
    const segment = element.children[index] as HTMLElement | undefined;
    if (!segment) return;
    const share = `${part.share * 100}%`;
    if (segment.style.width !== share) segment.style.width = share;
    const label = segment.firstElementChild as HTMLElement;
    const text = width === undefined || part.share * width > LABEL_MIN ? airPercent(part.score, view.total) : '';
    if (label.textContent !== text) label.textContent = text;
    (segment.lastElementChild as HTMLElement).hidden = !(index < parts.length - 1 && (width === undefined || part.share * width > SEAM_MIN));
  });
}
