import { useId, useLayoutEffect, useRef } from 'react';
import { hideTooltip, showTooltip, tooltipText } from '../tooltip.ts';
import type { RaceChartProps } from './RaceChart';
import { raceHeight, raceText } from './race.ts';
import type { RaceItem } from './race';
import type { PlaybackController } from '../../dashboard/playback';
import type { ArsenalModel, RaceInspection } from '../../draw/arsenal';
import { bitmapSurface, holdElement } from '../../draw/bitmapSurface.ts';
import { pipelineOf } from '../../dashboard/follow/pipeline.ts';
import { pageClock } from '../../dashboard/widgetClock.ts';
import { useFollowedWidget } from '../../dashboard/follow/useFollowed.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import { perfLog } from '../../shell/perf.ts';
import { validateNuclearArsenal, arsenalNotice, arsenalRows } from '../../widgets/nuclear-arsenal/nuclearArsenalData.ts';
import { countryRaceItem } from './countryRace.ts';

const RACE_AUDIT = -2;

type ExactRead = (hour: number) => Promise<unknown>;
type Model = Omit<ArsenalModel, 'width' | 'height' | 'dpr' | 'scroll'> & { exactRead?: undefined };

export default function DatedArsenal(props: RaceChartProps & { exactRead?: ExactRead; clock: PlaybackController; model: Model }) {
  const root = useRef<HTMLDivElement>(null), container = useRef<HTMLDivElement>(null), extent = useRef<HTMLDivElement>(null), ranking = useRef<HTMLParagraphElement>(null);
  const description = useRef<HTMLParagraphElement>(null), latest = useRef(props), update = useRef<() => void>(undefined);
  const owner = useId(), widgetKey = useWidgetKey();
  latest.current = props;
  const pipeline = pipelineOf(pageClock(props.clock));
  const pinned = props.model.follow === undefined && props.model.fixed !== undefined ? props.model.fixed + props.model.offset : null;
  const pipe = useFollowedWidget(pipeline, pipeline && props.active ? `${widgetKey}|${props.model.offset}` : null, { offset: props.model.offset, pin: pinned });
  useLayoutEffect(() => {
    const scroller = container.current, region = root.current;
    if (!scroller || !region || !props.active || !pipeline || pipe === null) return;
    const drawn = pipeline.held(pipe, 'race', () => {
      const canvas = document.createElement('canvas');
      canvas.tabIndex = 0; canvas.setAttribute('role', 'img'); canvas.setAttribute('aria-keyshortcuts', 'ArrowUp ArrowDown Home End Enter Space');
      canvas.dataset.surface = 'race'; canvas.dataset.placement = String(pipe);
      return { canvas, surface: bitmapSurface(canvas) };
    })!;
    const { canvas, surface } = drawn;
    canvas.setAttribute('aria-describedby', owner + '-ranking');
    scroller.prepend(canvas);
    let point: { x: number; y: number } | undefined, action: { x: number; y: number; key: string } | undefined, selected = -1, bound = '';
    const items = (): readonly RaceItem[] => (pipeline.view(pipe)?.fields?.items as readonly RaceItem[] | undefined) ?? [];
    const send = () => {
      const model = latest.current.model, { width, height } = canvas.getBoundingClientRect();
      if (!width || !height) return;
      const key = JSON.stringify([width, height, devicePixelRatio, scroller.scrollTop, model], (_, value: unknown) => value instanceof Map ? [...value] : value instanceof Set ? [...value] : value);
      if (key === bound) return;
      bound = key;
      pipeline.present(pipe, 'race');
      pipeline.bind(pipe, [surface], { surfaces: [{ width, height, dpr: devicePixelRatio }],
        ...model.follow === undefined && model.fixed !== undefined ? { dated: { h: model.fixed + model.offset, c: model.fixed + model.offset } } : {} },
        {}, { ...model, scroll: scroller.scrollTop });
      canvas.setAttribute('aria-label', latest.current.label);
    };
    update.current = send;
    const install = () => {
      const shown = pipeline.view(pipe);
      const held = !!shown?.dim;
      holdElement(region, held);
      canvas.style.visibility = shown?.mode === 'picture' ? '' : 'hidden';
      if (held) canvas.removeAttribute('aria-describedby'); else canvas.setAttribute('aria-describedby', owner + '-ranking');
      const listed = items(), p = latest.current;
      if (ranking.current) ranking.current.textContent = held ? '' : raceText(listed, p.name) || p.empty || '';
      if (extent.current) extent.current.style.height = `calc(${raceHeight(listed.length)}px - 100%)`;
      const inspected = shown?.tooltip as RaceInspection | null | undefined, item = inspected?.item;
      if (!item || shown?.dim) { hideTooltip(owner); if (description.current) description.current.textContent = ''; return; }
      selected = listed.findIndex(each => each.key === item.key);
      const asked = action?.key === `race:${item.key}` ? action : undefined;
      if (asked) action = undefined;
      const at = asked ?? point;
      if (!at) return;
      const content = p.tooltip?.(item.key, item);
      if (content) { showTooltip(owner, content, at.x, at.y, item.key); if (description.current) description.current.textContent = tooltipText(content); }
    };
    const perform = (kind: string, item: RaceItem, x: number, y: number) => {
      const shown = pipeline.view(pipe);
      if (shown?.mode !== 'picture' || shown.dim) return;
      const at = { ...item, hour: shown.through ?? shown.h ?? item.hour };
      if (kind === 'pick') latest.current.onPick?.(at.key, at);
      else latest.current.onContextMenu?.(at.key, x, y, at);
    };
    install();
    const stop = pipeline.onInstall(pipe, install);
    const pointer = (event: MouseEvent) => {
      const b = canvas.getBoundingClientRect();
      point = { x: event.clientX, y: event.clientY };
      if (event.type === 'contextmenu') event.preventDefault();
      const act = event.type === 'pointermove' ? undefined : { kind: event.type === 'click' ? 'pick' : 'menu', x: event.clientX, y: event.clientY };
      pipeline.pointer(pipe, 0, [event.clientX - b.left, event.clientY - b.top], act && (key => {
        const item = key === null ? undefined : items().find(each => `race:${each.key}` === key);
        if (item) perform(act.kind, item, act.x, act.y);
      }));
    };
    const leave = () => { point = undefined; pipeline.pointer(pipe, 0, null); pipeline.inspect(pipe, null); hideTooltip(owner); };
    const choose = (index: number, kind: string) => {
      const listed = items(), b = canvas.getBoundingClientRect();
      const item = listed[Math.min(listed.length - 1, Math.max(0, index))];
      if (!item) return;
      selected = listed.indexOf(item);
      if (kind !== 'tip') { perform(kind, item, b.right, b.top + b.height / 2); return; }
      action = { x: b.right, y: b.top + b.height / 2, key: `race:${item.key}` };
      if (pipeline.inspecting(pipe) === `race:${item.key}`) install();
      else pipeline.inspect(pipe, `race:${item.key}`);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { selected = -1; pipeline.inspect(pipe, null); hideTooltip(owner); return; }
      const kind = e.key === 'Enter' || e.key === ' ' ? 'pick' : e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey) ? 'menu' : 'tip';
      const index = e.key === 'Home' ? 0 : e.key === 'End' ? Number.MAX_SAFE_INTEGER : e.key === 'ArrowDown' ? selected + 1 : e.key === 'ArrowUp' ? Math.max(0, selected - 1) : selected;
      if (kind === 'tip' && !['Home', 'End', 'ArrowDown', 'ArrowUp'].includes(e.key) || index < 0) return;
      e.preventDefault(); choose(index, kind);
    };
    const focus = () => choose(Math.max(0, selected), 'tip');
    const blur = () => { point = undefined; pipeline.inspect(pipe, null); hideTooltip(owner); };
    const resize = new ResizeObserver(send); resize.observe(canvas);
    canvas.addEventListener('pointermove', pointer); canvas.addEventListener('click', pointer); canvas.addEventListener('contextmenu', pointer);
    canvas.addEventListener('focus', focus); canvas.addEventListener('blur', blur); canvas.addEventListener('pointerleave', leave); canvas.addEventListener('keydown', key);
    scroller.addEventListener('scroll', send, { passive: true });
    send();
    return () => {
      stop(); resize.disconnect(); scroller.removeEventListener('scroll', send); hideTooltip(owner); holdElement(region, false);
      canvas.removeEventListener('pointermove', pointer); canvas.removeEventListener('click', pointer); canvas.removeEventListener('contextmenu', pointer);
      canvas.removeEventListener('focus', focus); canvas.removeEventListener('blur', blur); canvas.removeEventListener('pointerleave', leave); canvas.removeEventListener('keydown', key);
      canvas.remove(); update.current = undefined;
    };
  }, [owner, pipeline, pipe, props.active, props.resetKey]);
  useLayoutEffect(() => { update.current?.(); }, [props.model]);
  const auditable = !!props.exactRead;
  useLayoutEffect(() => {
    if (!perfLog || !pipeline || pipe === null || !auditable) return;
    pipeline.audit(pipe, RACE_AUDIT, fields => fields.audit as readonly { row: string; key: string; value?: string }[] | undefined);
    perfLog.reader(pipe, RACE_AUDIT, async hour => {
      const read = latest.current.exactRead, model = latest.current.model;
      if (!read) return null;
      const reply = validateNuclearArsenal(await read(hour)), values = new Map<string, string>();
      for (const row of arsenalRows(reply, hour, model)) {
        const item = countryRaceItem(row, model.colours, () => model.tokens.accent, model.tokens.muted);
        values.set(`${item.key}:total`, item.label);
        for (const part of item.segments) if (part.id !== 'total') values.set(`${item.key}:${part.id}`, String(part.value));
      }
      if (!values.size) values.set('empty:notice', arsenalNotice(reply, hour));
      return { values, ranked: true };
    });
    return () => pipeline.audit(pipe, RACE_AUDIT, null);
  }, [pipeline, pipe, auditable]);
  return <div ref={root} className="race-chart"><div ref={container} className="race-scroll themed-scrollbar">
    <div ref={extent} className="race-extent" style={{ height: `calc(${raceHeight(props.items.length)}px - 100%)` }} /></div>
    <p ref={ranking} id={owner + '-ranking'} className="sr-only">{raceText(props.items, props.name) || props.empty}</p>
    <p ref={description} className="sr-only" aria-live="polite" /></div>;
}
