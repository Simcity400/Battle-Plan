import { useId, useLayoutEffect, useRef } from 'react';
import { clockGeneration, drawClock, drawPost, onDraw, surfaceId } from '../../dashboard/pulled.ts';
import { perfLog } from '../../shell/perf.ts';
import { measuredWidget } from '../../dashboard/widgetPerf.ts';
import { hideTooltip, showTooltip, tooltipText } from '../tooltip.ts';
import type { RaceChartProps } from './RaceChart';
import { raceHeight, raceText } from './race.ts';
import type { PlaybackController } from '../../dashboard/playback';
import type { ArsenalModel, RaceInspection, RaceTarget } from '../../draw/arsenal';
import { arsenalNotice, arsenalRows, validateNuclearArsenal } from '../../widgets/nuclear-arsenal/nuclearArsenalData.ts';
import { countryRaceItem } from './countryRace.ts';
import { noticeMatchesAnchor, shownTime } from '../../draw/anchorGeneration.ts';
import { heldPicture } from '../../draw/held.ts';

type ExactRead = (hour: number) => Promise<unknown>;
export default function DatedArsenal(props: RaceChartProps & { exactRead?: ExactRead; clock: PlaybackController; model: Omit<ArsenalModel, 'width' | 'height' | 'dpr' | 'scroll'> & { exactRead?: undefined } }) {
  const root = useRef<HTMLDivElement>(null), container = useRef<HTMLDivElement>(null), extent = useRef<HTMLDivElement>(null), ranking = useRef<HTMLParagraphElement>(null);
  const description = useRef<HTMLParagraphElement>(null), latest = useRef(props), update = useRef<() => void>(undefined);
  const owner = useId(), widget = measuredWidget();
  latest.current = props;
  useLayoutEffect(() => {
    const scroller = container.current;
    const region = root.current;
    if (!scroller || !region || !props.active) return;
    const canvas = document.createElement('canvas');
    canvas.tabIndex = 0; canvas.setAttribute('role', 'img'); canvas.setAttribute('aria-describedby', owner + '-ranking');
    canvas.setAttribute('aria-keyshortcuts', 'ArrowUp ArrowDown Home End Enter Space');
    scroller.prepend(canvas);
    const offscreen = canvas.transferControlToOffscreen(), id = surfaceId(), clock = props.clock;
    const stopClock = drawClock(clock);
    let point: { x: number; y: number } | undefined;
    let version = 0, ready = false, first = true, selected = -1, ticket = 0;
    let resolve: ((p?: RaceInspection) => void) | undefined;
    const visible = (show: boolean) => {
      canvas.style.visibility = show ? '' : 'hidden';
      if (ready !== show) perfLog?.ink(id, show);
      ready = show;
    };
    const settle = () => { resolve?.(); resolve = undefined; hideTooltip(owner); };
    const clear = () => { held.drop(); visible(false); point = undefined; selected = -1; settle();
      if (ranking.current) ranking.current.textContent = ''; if (description.current) description.current.textContent = ''; };
    const held = heldPicture(clock, region, () => latest.current.model.fixed === undefined, clear, settle);
    const send = () => {
      const model = latest.current.model, { width, height } = canvas.getBoundingClientRect();
      if (!width || !height) { clear(); return; }
      const message = { type: 'race' as const, id, owner: ++version, model: { ...model, view: 'nuclear-arsenal' as const, widget: widget ?? undefined, width, height, dpr: devicePixelRatio, scroll: scroller.scrollTop } };
      const { exactRead } = latest.current;
      if (exactRead) perfLog?.reader(id, version, async hour => {
        const reply = validateNuclearArsenal(await exactRead(hour));
        const values = new Map<string, string>();
        for (const row of arsenalRows(reply, hour, model)) {
          const item = countryRaceItem(row, model.colours, () => model.tokens.accent, model.tokens.muted);
          values.set(item.key + ':total', item.label);
          for (const part of item.segments) if (part.id !== 'total') values.set(item.key + ':' + part.id, String(part.value));
        }
        if (!values.size) values.set('empty:notice', arsenalNotice(reply, hour));
        return { values, ranked: true };
      });
      if (widget) { canvas.dataset.placement = String(id); canvas.dataset.surface = 'nuclear-arsenal'; }
      drawPost(first ? { ...message, canvas: offscreen } : message, first ? [offscreen] : []);
      if (point) {
        const b = canvas.getBoundingClientRect();
        drawPost({ type: 'racePointer', id, owner: version, point: { x: point.x - b.left, y: point.y - b.top } });
      }
      first = false; canvas.setAttribute('aria-label', latest.current.label);
    };
    update.current = send;
    const stop = onDraw(m => {
      if (m.type === 'failed' || m.type === 'followFailure') { clear(); return; }
      if ((m.type !== 'racePresent' && m.type !== 'raceInspection' && m.type !== 'raceHover') || m.id !== id || m.owner !== version || m.seek !== clock.getSnapshot().seekSequence ||
          (m.type === 'racePresent' && !noticeMatchesAnchor(m.generation, clockGeneration(clock)))) return;
      if (m.type === 'raceHover') {
        if (!point) return;
        const content = !held.stale && m.item && latest.current.tooltip?.(m.item.key, m.item);
        if (content && m.item) showTooltip(owner, content, point.x, point.y, m.item.key);
        else hideTooltip(owner);
        if (description.current) description.current.textContent = content ? tooltipText(content) : '';
        return;
      }
      if (m.type === 'raceInspection') { if (m.request === ticket) { resolve?.(m.picture); resolve = undefined; } return; }
      const clockHour = clock.positionAt(shownTime(m.shown));
      const ahead = latest.current.model.fixed === undefined && m.head !== undefined && Math.floor(m.head) > Math.floor(clockHour);
      if (m.hour === null && m.waiting && held.stale && !ahead) return;
      if (m.hour === null || ahead || m.hour > Math.floor(latest.current.model.fixed ?? clockHour)) clear();
      else { held.present(m.holdsThrough); visible(true); }
    });
    const inspect = (target?: RaceTarget) => new Promise<RaceInspection | undefined>(done => {
      resolve?.(); if (!ready || held.stale) { done(undefined); return; }
      resolve = done; drawPost({ type: 'inspectRace', id, owner: version, request: ++ticket, target });
    });
    const act = async (target: RaceTarget | undefined, action: string, x: number, y: number) => {
      const pic = await inspect(target), p = latest.current;
      if (!pic) return;
      if (ranking.current) ranking.current.textContent = raceText(pic.items, p.name) || p.empty || '';
      if (extent.current) extent.current.style.height = `calc(${raceHeight(pic.items.length)}px - 100%)`;
      if (!pic.item) { hideTooltip(owner); return; }
      const item = pic.item;
      selected = pic.items.findIndex(i => i.key === item.key);
      if (action === 'pick') p.onPick?.(item.key, item);
      else if (action === 'menu') p.onContextMenu?.(item.key, x, y, item);
      else {
        const content = p.tooltip?.(item.key, item);
        if (content) { showTooltip(owner, content, x, y, item.key); if (description.current) description.current.textContent = tooltipText(content); }
      }
    };
    const pointer = (event: MouseEvent) => {
      const b = canvas.getBoundingClientRect();
      if (event.type === 'pointermove') {
        point = { x: event.clientX, y: event.clientY };
        drawPost({ type: 'racePointer', id, owner: version, point: { x: point.x - b.left, y: point.y - b.top } });
        return;
      }
      if (event.type === 'contextmenu') event.preventDefault();
      void act({ x: event.clientX - b.left, y: event.clientY - b.top }, event.type === 'click' ? 'pick' : event.type === 'contextmenu' ? 'menu' : 'tip', event.clientX, event.clientY);
    };
    const leave = () => { point = undefined; drawPost({ type: 'racePointer', id, owner: version }); ++ticket; resolve?.(); resolve = undefined; hideTooltip(owner); };
    const focus = () => { void act(undefined, 'read', 0, 0); };
    const key = (e: KeyboardEvent) => {
      const b = canvas.getBoundingClientRect();
      if (e.key === 'Escape') { selected = -1; leave(); return; }
      const action = e.key === 'Enter' || e.key === ' ' ? 'pick' : e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey) ? 'menu' : 'tip';
      const index = e.key === 'Home' ? 0 : e.key === 'End' ? Number.MAX_SAFE_INTEGER : e.key === 'ArrowDown' ? selected + 1 : e.key === 'ArrowUp' ? Math.max(0, selected - 1) : selected;
      if (action === 'tip' && !['Home','End','ArrowDown','ArrowUp'].includes(e.key) || index < 0) return;
      e.preventDefault(); void act({ index }, action, b.right, b.top + b.height / 2);
    };
    const anchor = clock.onDiscard(() => { if (latest.current.model.fixed === undefined) clear(); });
    const resize = new ResizeObserver(send); resize.observe(canvas);
    canvas.addEventListener('pointermove', pointer); canvas.addEventListener('click', pointer); canvas.addEventListener('contextmenu', pointer);
    canvas.addEventListener('focus', focus); canvas.addEventListener('blur', leave); canvas.addEventListener('pointerleave', leave); canvas.addEventListener('keydown', key);
    scroller.addEventListener('scroll', send, { passive: true });
    clear(); send();
    return () => { clear(); stop(); stopClock(); anchor(); held.close(); resize.disconnect(); scroller.removeEventListener('scroll', send); canvas.remove();
      drawPost({ type: 'closeRace', id }); update.current = undefined; };
  }, [owner, props.clock, props.active, props.model.follow, props.model.fixed, props.resetKey, widget]);
  useLayoutEffect(() => { update.current?.(); }, [props.model]);
  return <div ref={root} className="race-chart"><div ref={container} className="race-scroll themed-scrollbar">
    <div ref={extent} className="race-extent" style={{ height: `calc(${raceHeight(props.items.length)}px - 100%)` }} /></div>
    <p ref={ranking} id={owner + '-ranking'} className="sr-only">{raceText(props.items, props.name) || props.empty}</p>
    <p ref={description} className="sr-only" aria-live="polite" /></div>;
}
