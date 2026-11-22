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
      canvas.dataset.surface = 'race';
      return { canvas, surface: bitmapSurface(canvas) };
    })!;
    const { canvas, surface } = drawn;
    canvas.setAttribute('aria-describedby', owner + '-ranking');
    scroller.prepend(canvas);
    let point: { x: number; y: number } | undefined, action: { kind: string; x: number; y: number } | undefined, selected = -1;
    const items = (): readonly RaceItem[] => (pipeline.view(pipe)?.fields?.items as readonly RaceItem[] | undefined) ?? [];
    const send = () => {
      const model = latest.current.model, { width, height } = canvas.getBoundingClientRect();
      if (!width || !height) return;
      pipeline.present(pipe, 'race');
      pipeline.bind(pipe, [surface], { surfaces: [{ width, height, dpr: devicePixelRatio }],
        ...model.follow === undefined && model.fixed !== undefined ? { dated: { h: model.fixed + model.offset, c: model.fixed + model.offset } } : {} },
        {}, { ...model, scroll: scroller.scrollTop });
      canvas.setAttribute('aria-label', latest.current.label);
    };
    update.current = send;
    const install = () => {
      const shown = pipeline.view(pipe);
      holdElement(region, !!shown?.dim);
      canvas.style.visibility = shown?.mode === 'picture' ? '' : 'hidden';
      const listed = items(), p = latest.current;
      if (ranking.current) ranking.current.textContent = raceText(listed, p.name) || p.empty || '';
      if (extent.current) extent.current.style.height = `calc(${raceHeight(listed.length)}px - 100%)`;
      const inspected = shown?.tooltip as RaceInspection | null | undefined, item = inspected?.item;
      if (!item || shown?.dim) { if (!point) hideTooltip(owner); return; }
      selected = listed.findIndex(each => each.key === item.key);
      const at = action ?? (point && { kind: 'tip', ...point });
      action = undefined;
      if (!at) return;
      if (at.kind === 'pick') p.onPick?.(item.key, item);
      else if (at.kind === 'menu') p.onContextMenu?.(item.key, at.x, at.y, item);
      else {
        const content = p.tooltip?.(item.key, item);
        if (content) { showTooltip(owner, content, at.x, at.y, item.key); if (description.current) description.current.textContent = tooltipText(content); }
      }
    };
    install();
    const stop = pipeline.onInstall(pipe, install);
    const pointer = (event: MouseEvent) => {
      const b = canvas.getBoundingClientRect();
      point = { x: event.clientX, y: event.clientY };
      if (event.type === 'contextmenu') event.preventDefault();
      if (event.type !== 'pointermove') action = { kind: event.type === 'click' ? 'pick' : 'menu', x: event.clientX, y: event.clientY };
      pipeline.pointer(pipe, 0, [event.clientX - b.left, event.clientY - b.top]);
    };
    const leave = () => { point = undefined; pipeline.pointer(pipe, 0, null); pipeline.inspect(pipe, null); hideTooltip(owner); };
    const choose = (index: number, kind: string) => {
      const listed = items(), b = canvas.getBoundingClientRect();
      const item = listed[Math.min(listed.length - 1, Math.max(0, index))];
      if (!item) return;
      action = { kind, x: b.right, y: b.top + b.height / 2 };
      pipeline.inspect(pipe, `race:${item.key}`);
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
  return <div ref={root} className="race-chart"><div ref={container} className="race-scroll themed-scrollbar">
    <div ref={extent} className="race-extent" style={{ height: `calc(${raceHeight(props.items.length)}px - 100%)` }} /></div>
    <p ref={ranking} id={owner + '-ranking'} className="sr-only">{raceText(props.items, props.name) || props.empty}</p>
    <p ref={description} className="sr-only" aria-live="polite" /></div>;
}
