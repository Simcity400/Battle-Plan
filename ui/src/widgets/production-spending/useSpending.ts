import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo, useRef } from 'react';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { WidgetSpec } from '../../dashboard/model';
import { windowOf, windowsOf } from '../../dashboard/heldWindows';
import { useFrameWindows } from '../army-size/frameWindows.ts';
import { useHeadHour } from '../army-size/headHour.ts';
import type { ProductionSpendingReply } from '../../generated/dashboard';
import { mergeSpending } from './spendingData';
import type { SpendingData } from './spendingData';
import { productRequest } from '../../dashboard/request.ts';

export function useSpending(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, repairs: boolean,
  validate: (data: unknown) => ProductionSpendingReply, key: (hour: number) => string, whole = false) {
  const following = active && widget.time.mode === 'follow';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const headAt = useMemo(() => following && track ? (position: number) => {
    const at = resolveWindow(widget, track, runtime.tracks, position)?.lastHour;
    return at === undefined || at < track.branch.firstHour || at > track.branch.lastHour ? null : at;
  } : null, [following, widget, track, runtime.tracks]);
  const keys = useRef({ key, headAt });
  keys.current = { key, headAt };
  const playbackHour = useHeadHour(runtime.clock, following, position => {
    const at = keys.current.headAt?.(position);
    return at == null ? 'outside' : keys.current.key(at);
  });
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour;
  const range = widget.time.mode === 'range';
  const from = range ? window?.firstHour ?? null : null;
  const end = range ? hour : track?.branch.lastHour;
  const windowsAt = (position: number, ahead: boolean): number[] => {
    if (!track || hour == null || end == null) return [];
    const at = keys.current.headAt?.(position) ?? hour;
    if (whole) return windowsOf(from ?? track.branch.firstHour, Math.max(end, at), ahead);
    const hours = [at, end, ...(from == null ? [] : [from, from - 1])].filter(value => value >= track.branch.firstHour && value <= track.branch.lastHour);
    return [...new Set([...hours.map(windowOf), ...(ahead ? [windowOf(at) + 1] : [])])].sort((a, b) => a - b);
  };
  const identity = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, repairs]) : null;
  const subscribeClock = useMemo(() => (changed: () => void) => {
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const held = useFrameWindows<ProductionSpendingReply>(runtime.client, identity, index => (productRequest({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'production-spending', params: { branchId: track!.branch.id, country: countryFromKey(tag!), repairs, window: index } })),
  () => windowsAt(runtime.clock.getSnapshot().position, following), subscribeClock, active, validate, track?.source);
  const needed = windowsAt(runtime.clock.getSnapshot().position, false);
  const error = needed.map(index => held.errors.get(index)).find(failure => failure !== undefined);
  const complete = needed.length > 0 && needed.every(index => held.data.has(index));
  const data: SpendingData | undefined = useMemo(() => complete ? mergeSpending(held.data) ?? undefined : undefined, [complete, held.data]);
  const retry = held.retry;
  return { track, tag, data, error, retry, window, hour, from, end, headAt };
}
