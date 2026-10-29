import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import * as requests from '../../dashboard/batch5Requests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { loadTransientProduct, useProduct } from '../../dashboard/products';
import { finalPreparedReply } from '../../dashboard/preparedPrefix';
import { useContentRevision } from '../../dashboard/contentRevision';
import { validateWeather } from './weatherData';
import type { WeatherReply } from './weatherData';
import { OPTIONS } from './definition';
import { weatherDayRequest, weatherRequest, weatherShow } from '../../dashboard/widgetRequests';

/** Column budgets step by this many pixels, so a small resize asks nothing again. */
export const WIDTH_STEP = 256;
export const stepWidth = (pixels: number) => Math.ceil(Math.max(1, pixels) / WIDTH_STEP) * WIDTH_STEP;
/** How long the view rests after a zoom, pan or resize before its days are asked again. */
export const VIEW_SETTLE_MS = 150;
/** How long the pointer rests on a folded column's day before that day is read. */
export const DWELL_MS = 100;
/** How long a day still being prepared is shown before it is read again. */
export const PREPARING_REREAD_MS = 2000;
/** The finished days a widget keeps read, most recent last. */
export const RECENT_DAYS = 8;

/** The window's width in pixels, until the chart reports its plot's. */
function useWindowWidth(): number {
  const [width, setWidth] = useState(() => stepWidth(globalThis.innerWidth ?? 1));
  useEffect(() => {
    const update = () => setWidth(stepWidth(globalThis.innerWidth));
    globalThis.addEventListener('resize', update);
    return () => globalThis.removeEventListener('resize', update);
  }, []);
  return width;
}

/** The shown days' prepared weather, sampled once a day and folded into at most one
 * column per pixel of the plot (`pixels`, else the window's), for the chosen view.
 * Days that fit keep one a column. The last reply stays shown while the next loads. */
export function useWeather(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null, pixels: number | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const { show, chosen } = weatherShow(widget);
  const window = useWindowWidth();
  const request = weatherRequest(widget, track, extent, pixels ?? window);
  const product = useProduct<WeatherReply>(runtime.client, request, active, validateWeather, track?.source);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, show, chosen]);
  const kept = useRef<{ client: unknown; scope: string; data: WeatherReply } | null>(null);
  if (product.data) kept.current = { client: runtime.client, scope, data: product.data };
  const data = product.data ?? (kept.current?.client === runtime.client && kept.current.scope === scope && !product.error ? kept.current.data : undefined);
  const modifier = show === 'modifiers' ? chosen ?? data?.modifiers[0]?.id ?? null : null;
  return { track, options, show, modifier, ...product, data };
}

/** The weather of the hours the chart shows: `extent` until the chart reports its
 * view, then the hours in view at the plot's width, once the view has settled. */
export function useWeatherInView(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const owner = JSON.stringify([track?.source.id, track?.branch.id, extent?.[0], extent?.[1]]);
  const [view, setView] = useState<{ owner: string; from: number; to: number; pixels: number } | null>(null);
  const settle = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(settle.current), []);
  const onSummaryView = useCallback((next: { from: number; to: number; width: number }) => {
    clearTimeout(settle.current);
    settle.current = setTimeout(() => setView(previous => {
      const shown = { owner, from: Math.floor(next.from), to: Math.ceil(next.to), pixels: stepWidth(next.width) };
      return previous && previous.owner === shown.owner && previous.from === shown.from && previous.to === shown.to && previous.pixels === shown.pixels ? previous : shown;
    }), VIEW_SETTLE_MS);
  }, [owner]);
  const current = view?.owner === owner ? view : null;
  const asked: [number, number] | null = extent && current ? [Math.max(extent[0], current.from), Math.min(extent[1], current.to)] : null;
  const weather = useWeather(widget, runtime, active, asked && asked[0] <= asked[1] ? asked : extent, current?.pixels ?? null);
  return { ...weather, onSummaryView };
}

interface DayRead { data?: WeatherReply; error?: string; at: number }

/** Folded columns' days read exactly, once the pointer rests on one: each read is
 * transient, outside the shared product cache. Only finished days are kept, the
 * most recent `RECENT_DAYS`; a day still being prepared is read again after a
 * while for as long as the pointer stays on it, and a failed read once the pointer
 * comes back to it. */
export function useWeatherDays(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, dayHours: number) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  // Days read under other content folders, or by another client, are not this widget's.
  const content = useContentRevision(runtime.client);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, weatherShow(widget), dayHours, content]);
  const cache = useRef({ scope, client: runtime.client, days: new Map<number, DayRead>(), last: null as { day: number; read: DayRead } | null });
  if (cache.current.scope !== scope || cache.current.client !== runtime.client) cache.current = { scope, client: runtime.client, days: new Map(), last: null };
  const [revision, bump] = useReducer((value: number) => value + 1, 0);
  const wanted = useRef<{ day: number; timer?: ReturnType<typeof setTimeout>; controller?: AbortController } | null>(null);
  const forget = () => { if (wanted.current) { clearTimeout(wanted.current.timer); wanted.current.controller?.abort(); } wanted.current = null; };
  // The day under the pointer, or null off the chart: a failure landing after it left is not kept.
  const pointer = useRef<number | null>(null);
  useEffect(() => forget, [scope, active, runtime.client]);
  const want = useCallback((day: number) => {
    const owner = cache.current;
    pointer.current = day;
    // A failed read is forgotten once the pointer leaves its day.
    if (owner.last && owner.last.day !== day && owner.last.read.error !== undefined) owner.last = null;
    if (!active || !track || owner.days.has(day) || wanted.current?.day === day) return;
    const last = owner.last?.day === day ? owner.last.read : null;
    if (last?.error !== undefined) return;
    forget();
    const entry: NonNullable<typeof wanted.current> = { day };
    wanted.current = entry;
    // A day read while still being prepared is read again once that read is old enough.
    entry.timer = setTimeout(() => {
      const controller = new AbortController();
      entry.controller = controller;
      void loadTransientProduct<unknown>(runtime.client, weatherDayRequest(widget, track, day, dayHours), controller.signal, track.source).then(
        value => ({ data: validateWeather(value), at: Date.now() }), error => ({ error: error instanceof Error ? error.message : String(error), at: Date.now() }))
        .then((read: DayRead) => {
          if (controller.signal.aborted || cache.current !== owner) return;
          if (read.data && finalPreparedReply(read.data)) {
            owner.days.delete(day);
            owner.days.set(day, read);
            while (owner.days.size > RECENT_DAYS) owner.days.delete(owner.days.keys().next().value!);
            if (owner.last?.day === day) owner.last = null;
          } else if (read.error === undefined || pointer.current === day) owner.last = { day, read };
          if (wanted.current === entry) wanted.current = null;
          bump();
        });
    }, last ? Math.max(DWELL_MS, PREPARING_REREAD_MS - (Date.now() - last.at)) : DWELL_MS);
    // The widget and its track own the request; the scope key covers what it reads.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, scope, runtime.client]);
  const get = useCallback((day: number): DayRead | undefined => {
    const owner = cache.current;
    return owner.days.get(day) ?? (owner.last?.day === day ? owner.last.read : undefined);
  }, [scope, revision, runtime.client]); // eslint-disable-line react-hooks/exhaustive-deps
  /** The pointer is on `day`, or off the chart (null): a failed read of any other day is
   * forgotten, and a read of another day not yet started is dropped. */
  const hover = useCallback((day: number | null) => {
    const owner = cache.current;
    pointer.current = day;
    if (owner.last && owner.last.day !== day && owner.last.read.error !== undefined) owner.last = null;
    if (wanted.current && wanted.current.day !== day && !wanted.current.controller) forget();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  return { get, want, hover };
}

/** Regions, theatres and modifiers as the recording and its content name them. */
export function useWeatherChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather',
    params: requests.weatherHistory(track.branch.id, track.branch.lastHour, track.branch.lastHour, 'conditions', null) } : null;
  const { data } = useProduct<WeatherReply>(runtime.client, request, true, validateWeather, track?.source);
  return {
    regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })),
    theatres: (data?.theatres ?? []).map(theatre => ({ value: setToken(theatre.id), label: theatre.name })),
    modifier: (data?.modifiers ?? []).map(modifier => ({ value: setToken(modifier.id), label: modifier.name })),
  };
}
