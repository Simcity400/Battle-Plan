import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode, RefObject } from 'react';
import { formatDate, formatDateTime, formatDay, gameDate } from '../dashboard/calendar';
import type { WidgetRuntime } from '../dashboard/definitions';
import { useMapState, useMapsHidden, type MapTarget } from '../dashboard/geometry';
import type { BoundTrack } from '../dashboard/model';
import { DetailWindowHost, useDetailWindowClose } from '../families/host/DetailWindowHost.tsx';
import { subscribeTokens } from '../charts/tokens.ts';
import { parseColor } from '../shell/appearance/color.ts';
import { mapFrameOnScreen, mapRequests } from '../widgets/map/presentation';
import { FailureLine } from './FailureLine';
import { Icon } from './Icons';
import { Segmented } from './Segmented';
import { BattleClock, issueSequence, sharedPause, type BattleSpan } from './battleClock.ts';
import './battleViewer.css';

export type { BattleSpan } from './battleClock.ts';

type ViewerRuntime = Pick<WidgetRuntime, 'maps' | 'clock' | 'client' | 'nativeHidden'>;
export type BattleView = 'battle' | 'map' | 'details';

const VIEWS = [{ value: 'battle', label: 'Battle' }, { value: 'map', label: 'Map' }, { value: 'details', label: 'Details' }] as const;
const ALL_VIEWS: readonly BattleView[] = ['battle', 'map', 'details'];
const DAY = 24;

export interface BattleViewerProps {
  readonly subject: string;
  readonly title: string;
  readonly day: ReactNode;
  readonly closeLabel: string;
  readonly anchor: RefObject<HTMLElement | null>;
  readonly returnFocus?: string;
  readonly runtime: ViewerRuntime;
  readonly track: BoundTrack;
  readonly province: number | null;
  readonly span: BattleSpan | null;
  readonly unplayable: ReactNode;
  readonly gaps: readonly BattleSpan[];
  readonly fallbackHour: number;
  readonly startHour?: number;
  readonly sides: (hour: number) => ReactNode;
  readonly battle: (hour: number, clock: BattleClock | null) => ReactNode;
  readonly details?: (hour: number) => ReactNode;
  readonly views?: readonly BattleView[];
  readonly onClose: () => void;
}

const still = () => () => {};

function shortDay(hour: number): string {
  const date = gameDate(hour);
  return date ? formatDate(date).slice(0, -(String(date.year).length + 1)) : formatDay(hour);
}

function MapState({ icon, busy, children }: { readonly icon?: boolean; readonly busy?: boolean; readonly children: ReactNode }) {
  return <div className="bv-map-state" role="status" aria-busy={busy || undefined}>
    {icon && <span className="bv-map-icon">{busy ? <span className="spinner" /> : <Icon name="map" />}</span>}
    {children}
  </div>;
}

let opened = 0;

function BattleMap({ runtime, track, clock, province, fallbackHour }: {
  readonly runtime: ViewerRuntime;
  readonly track: BoundTrack;
  readonly clock: BattleClock | null;
  readonly province: number | null;
  readonly fallbackHour: number;
}) {
  const body = useRef<HTMLDivElement>(null);
  const id = useMemo(() => `battle-map-${++opened}`, []);
  const native = runtime.client.available && runtime.client.nativeMaps !== false;
  const placed = native && province !== null && !runtime.nativeHidden;
  const request = useMemo(() => mapRequests(runtime.clock), [runtime.clock]);
  const target = useCallback((): MapTarget | null => {
    if (province === null) return null;
    const position = clock ? clock.positionAt() : fallbackHour;
    const hour = Math.floor(position);
    if (hour < track.branch.firstHour || hour > track.branch.lastHour) return null;
    return request({
      options: { fog: false, country: track.selectedTag ?? null, countrySlot: track.selectedSlot ?? null, countryPinned: false, observed: null, pickerHidden: true },
      sourceId: track.source.id, path: track.source.path, revision: track.source.revision, branchId: track.branch.id,
      seekSequence: clock?.seekSequence ?? 0, playing: clock?.playing ?? false, hoursPerSecond: clock?.rate ?? 1,
      hour, fraction: position - hour, focusProvince: province,
    });
  }, [track, clock, province, fallbackHour, request]);
  useLayoutEffect(() => placed && body.current ? runtime.maps.register(id, body.current, target) : undefined, [placed, runtime.maps, id, target]);
  useLayoutEffect(() => () => runtime.maps.forget(id), [runtime.maps, id]);
  useEffect(() => {
    if (!clock) return;
    let seen = clock.sequence;
    return clock.subscribe(() => {
      if (clock.sequence === seen) return;
      seen = clock.sequence;
      runtime.maps.retarget();
    });
  }, [clock, runtime.maps]);
  const state = useMapState(runtime.maps, id);
  const hidden = useMapsHidden(runtime.maps);
  const shown = placed && !hidden && mapFrameOnScreen(state);
  const notes = shown ? state?.notes ?? [] : [];
  return <>{notes.length > 0 && <div className="bv-battle-notes">{notes.map(note => <span key={note} className="bv-missing"><i className="bv-hatch" aria-hidden="true" />{note}</span>)}</div>}<div className="bv-map">
    <div className="bv-map-surface" ref={body} data-map-current={shown || undefined}>
      {!native ? <MapState icon><strong>Map available in the desktop app.</strong></MapState>
        : province === null ? <MapState><span className="bv-missing"><i className="bv-hatch" aria-hidden="true" />Place not recorded</span>
          <p>The recording does not name the battle’s province, so the map cannot be placed.</p></MapState>
          : runtime.nativeHidden || hidden ? <MapState icon><strong>Map hidden for layout controls.</strong></MapState>
            : state?.error ? <MapState><FailureLine message="The battle map couldn’t be drawn." detail={state.error} onRetry={() => runtime.maps.retry(id)} /></MapState>
              : shown ? null : <MapState icon busy><strong>Preparing the map…</strong></MapState>}
    </div>
  </div></>;
}

type Rgb = [number, number, number];

function rgbOf(value: string): Rgb | null {
  const rgb = parseColor(value);
  return rgb ? [Math.round(rgb.r), Math.round(rgb.g), Math.round(rgb.b)] : null;
}

export interface BattleLead { readonly slot: number; readonly tag: string }

export function BattleGameWindow({ runtime, track, clock, kind = 'land', combat, result = null, lead, live, until, ended }: {
  readonly runtime: ViewerRuntime;
  readonly track: BoundTrack;
  readonly clock: BattleClock;
  readonly kind?: 'land' | 'naval';
  readonly combat: string | null;
  readonly result?: { readonly handle: string; readonly from: number } | null;
  readonly lead: BattleLead;
  readonly live: readonly (readonly [number, number])[];
  readonly until: number | null;
  readonly ended: ReactNode;
}) {
  const body = useRef<HTMLDivElement>(null);
  const id = useMemo(() => `battle-window-${++opened}`, []);
  const leading = useRef(lead);
  leading.current = lead;
  const state = useMapState(runtime.maps, id);
  const hidden = useMapsHidden(runtime.maps);
  const atEnd = ended !== null && until !== null && state?.hour != null && state.hour >= until && state.sequence >= clock.movedSequence;
  const [reached, setReached] = useState(() => ended !== null && until !== null && (clock.seenThrough ?? Number.NEGATIVE_INFINITY) >= until);
  useEffect(() => { if (state?.hour != null) clock.acknowledge(state.hour, state.sequence); }, [clock, state?.hour, state?.sequence]);
  const past = ended !== null;
  const ending = useRef(past);
  ending.current = past;
  const failed = !!state?.error;
  useEffect(() => { setReached(was => past && (was || atEnd || failed)); }, [past, atEnd, failed]);
  const native = runtime.client.available && runtime.client.nativeMaps !== false;
  const caught = past && (!native || reached || atEnd || failed);
  const placed = native && !runtime.nativeHidden && !caught;
  const request = useMemo(() => mapRequests({ issue: issueSequence }), []);
  const target = useCallback((): MapTarget | null => {
    const hour = ending.current ? Math.min(clock.getHour(), until ?? Number.MAX_SAFE_INTEGER) : clock.getHour();
    const style = body.current && getComputedStyle(body.current);
    const canvas = style && rgbOf(style.backgroundColor), text = style && rgbOf(style.color);
    if (!canvas || !text || hour < track.branch.firstHour || hour > track.branch.lastHour) return null;
    return request({
      options: { fog: true, country: track.selectedTag ?? null, countrySlot: track.selectedSlot ?? null, countryPinned: false, observed: null, pickerHidden: true },
      sourceId: track.source.id, path: track.source.path, revision: track.source.revision, branchId: track.branch.id,
      seekSequence: clock.jumpSequence, playing: clock.playing, hoursPerSecond: clock.rate, hour, fraction: 0,
      battle: { kind, ...(combat === null ? {} : { combat }), ...(result ? { result: { handle: result.handle, from: result.from } } : {}), country: leading.current.tag, countrySlot: leading.current.slot, playedFrom: clock.resumeFrom, live: live.map(([first, last]) => [first, last]), canvas, text },
    });
  }, [track, clock, kind, combat, result, until, live, request]);
  useLayoutEffect(() => placed && body.current ? runtime.maps.register(id, body.current, target, false) : undefined, [placed, runtime.maps, id, target]);
  useEffect(() => clock.subscribe(() => runtime.maps.retarget()), [clock, runtime.maps]);
  useEffect(() => { runtime.maps.retarget(); }, [past, runtime.maps]);
  useEffect(() => { runtime.maps.retarget(); }, [lead.slot, lead.tag, runtime.maps]);
  useEffect(() => subscribeTokens(() => runtime.maps.retarget()), [runtime.maps]);
  const close = useDetailWindowClose();
  const asked = state?.closeRequests ?? 0;
  const answered = useRef(asked);
  useEffect(() => {
    if (asked > answered.current) close?.();
    answered.current = asked;
  }, [asked, close]);
  const keys = state?.keys;
  const played = useRef(0);
  useEffect(() => {
    for (const [order, key] of keys ?? []) {
      if (order <= played.current) continue;
      played.current = order;
      const [shift, name] = key.startsWith('Shift+') ? [true, key.slice(6)] : [false, key];
      if (name === 'Space') clock.toggle();
      else if (name === 'ArrowLeft' || name === 'ArrowRight') clock.step((name === 'ArrowLeft' ? -1 : 1) * (shift ? DAY : 1));
      else if (name === 'Home') clock.seek(clock.span.first);
      else if (name === 'End') clock.seek(clock.span.last);
      else if (name === 'Escape') close?.();
    }
  }, [keys, clock, close]);
  if (caught) return <>{ended}</>;
  const notes = state?.hour != null ? state.notes ?? [] : [];
  return <div className="bv-battle">
    <div className="bv-battle-surface" ref={body}>
      {!native ? <MapState><strong>The battle window is available in the desktop app.</strong></MapState>
        : runtime.nativeHidden || hidden ? <MapState><strong>Battle window hidden for layout controls.</strong></MapState>
          : state?.error ? <MapState><FailureLine message="The battle window couldn’t be drawn." detail={state.error} onRetry={() => runtime.maps.retry(id)} /></MapState>
            : state?.hour == null ? <MapState busy><strong>Reading the battle…</strong></MapState> : null}
    </div>
    <div className="bv-battle-notes">
      <span>Artwork from current installation</span>
      {notes.length > 0 && <span tabIndex={0} data-tooltip={notes.join('\n')}>{notes.length === 1 ? '1 note on what this window draws' : `${notes.length} notes on what this window draws`}</span>}
    </div>
  </div>;
}

function Transport({ clock, gaps, unplayable, fallbackHour }: {
  readonly clock: BattleClock | null;
  readonly gaps: readonly BattleSpan[];
  readonly unplayable: ReactNode;
  readonly fallbackHour: number;
}) {
  const subscribe = clock?.subscribe ?? still;
  const hour = useSyncExternalStore(subscribe, clock?.getHour ?? (() => fallbackHour));
  const playing = useSyncExternalStore(subscribe, clock?.getPlaying ?? (() => false));
  const scrub = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = scrub.current;
    if (!clock || !element) return;
    const { first, last } = clock.span;
    const paint = () => element.style.setProperty('--at', String(last > first ? (clock.positionAt() - first) / (last - first) : 1));
    paint();
    return clock.subscribeFrame(paint);
  }, [clock]);
  if (!clock) return <div className="bv-transport">
    <button type="button" className="bv-play" disabled aria-label="Play"><Icon name="play" /></button>
    <span className="bv-date">{formatDateTime(fallbackHour)}</span>
    <span className="bv-unplayable">{unplayable}</span>
  </div>;
  const { first, last } = clock.span;
  const length = Math.max(1, last - first);
  const keys = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (!event.shiftKey || (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight')) return;
    event.preventDefault();
    clock.step(event.key === 'ArrowLeft' ? -DAY : DAY);
  };
  return <div className="bv-transport">
    <button type="button" className="bv-play" aria-label={playing ? 'Pause' : 'Play'} disabled={last <= first}
      onClick={() => clock.toggle()}><Icon name={playing ? 'pause' : 'play'} /></button>
    <time className="bv-date">{formatDateTime(hour)}</time>
    <span className="bv-end">{shortDay(first)}</span>
    <div className="bv-scrub" ref={scrub}>
      <i className="bv-track" aria-hidden="true" />
      {gaps.map(gap => <i key={gap.first} className="bv-gap" aria-hidden="true"
        style={{ left: `${(Math.max(gap.first - 0.5, first) - first) / length * 100}%`, width: `${(Math.min(gap.last + 0.5, last) - Math.max(gap.first - 0.5, first)) / length * 100}%` }} />)}
      <i className="bv-fill" aria-hidden="true" />
      <i className="bv-knob" aria-hidden="true" />
      <input type="range" min={first} max={last} step={1} value={hour} aria-label="Battle time" aria-valuetext={`${formatDateTime(hour)}${gaps.some(gap => gap.first <= hour && hour <= gap.last) ? ' · not recorded' : ''}`}
        disabled={last <= first} onChange={event => clock.seek(Number(event.currentTarget.value))} onKeyDown={keys} />
    </div>
    <span className="bv-end">{shortDay(last)}</span>
  </div>;
}

export function BattleViewer(props: BattleViewerProps) {
  const { runtime, span, views = ALL_VIEWS } = props;
  const [view, setView] = useState<BattleView>('battle');
  const [viewing, setViewing] = useState(props.subject);
  if (viewing !== props.subject) {
    setViewing(props.subject);
    setView('battle');
  }
  const start = props.startHour;
  const clock = useMemo(() => span ? new BattleClock(span, Math.min(span.last, Math.max(span.first, start ?? span.first))) : null, [props.subject, span?.first, span?.last, start]);
  useEffect(() => () => clock?.dispose(), [clock]);
  const hour = useSyncExternalStore(clock?.subscribe ?? still, clock?.getHour ?? (() => props.fallbackHour));
  const page = runtime.clock;
  const pause = useMemo(() => sharedPause(page), [page]);
  const body = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const host = body.current?.closest<HTMLElement>('.battle-viewer');
    if (!host || !clock) return;
    const keys = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
      const target = event.target as Element;
      const move = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : 0;
      if (event.key === ' ') {
        if (target.closest('button, a, select, textarea, input:not([type="range"])')) return;
        clock.toggle();
      } else if (target.closest('select, textarea, input')) return;
      else if (move) clock.step(move * (event.shiftKey ? DAY : 1));
      else if (event.key === 'Home') clock.seek(clock.span.first);
      else if (event.key === 'End') clock.seek(clock.span.last);
      else return;
      event.preventDefault();
    };
    host.addEventListener('keydown', keys);
    return () => host.removeEventListener('keydown', keys);
  }, [clock]);
  return <DetailWindowHost open subject={props.subject} hour={hour} title={props.title} day={props.day} placement="center"
    anchor={props.anchor} returnFocus={props.returnFocus} pauseClock clock={pause} className="battle-viewer" closeLabel={props.closeLabel}
    headerActions={views.length > 1 ? <Segmented label="Battle view" value={view} options={VIEWS.filter(option => views.includes(option.value))} onChange={setView} /> : undefined} onClose={props.onClose}>
    <div className="bv-view" ref={body}>
      {props.sides(hour)}
      {view === 'battle' ? props.battle(hour, clock)
        : view === 'map' ? <BattleMap runtime={runtime} track={props.track} clock={clock} province={props.province} fallbackHour={props.fallbackHour} />
          : <div className="bv-details">{props.details?.(hour)}</div>}
      <Transport clock={clock} gaps={props.gaps} unplayable={props.unplayable} fallbackHour={props.fallbackHour} />
    </div>
  </DetailWindowHost>;
}
