import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo, useRef } from 'react';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { windowOf } from '../../dashboard/heldWindows.ts';
import { resolveWindow } from '../../dashboard/model';
import type { WidgetSpec } from '../../dashboard/model';
import { useFrameWindows } from '../army-size/frameWindows.ts';
import { useHeadHour } from '../army-size/headHour.ts';
import type { ProductionSpendingReply } from '../../generated/dashboard';
import { createWindowTracker, displayedSpendingWindows, prefetchSpendingWindows, selectSpendingState } from './spendingData';
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
    if (at == null) return 'outside';
    return `${windowOf(Math.floor(at))}:${keys.current.key(at)}`;
  });
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour;
  const range = widget.time.mode === 'range';
  const from = range ? window?.firstHour ?? null : null;
  const end = range ? hour : track?.branch.lastHour;
  const wantedTracker = useMemo(() => createWindowTracker(prefetchSpendingWindows), []);
  const neededTracker = useMemo(() => createWindowTracker(displayedSpendingWindows), []);
  const atFor = (position: number): number | null => {
    if (!track || hour == null) return null;
    return keys.current.headAt?.(position) ?? hour;
  };
  const wantedAt = (position: number): number[] => {
    const at = atFor(position);
    return !track || at == null ? [] : wantedTracker.get(track.branch, at, from, whole, repairs, following);
  };
  const neededAt = (position: number): number[] => {
    const at = atFor(position);
    return !track || at == null ? [] : neededTracker.get(track.branch, at, from, whole, repairs, false);
  };
  const identity = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, repairs]) : null;
  const subscribeClock = useMemo(() => (changed: () => void) => {
    const currentWindow = () => {
      const at = keys.current.headAt?.(runtime.clock.getSnapshot().position);
      return at == null ? null : windowOf(Math.floor(at));
    };
    let seenWindow = currentWindow();
    const onFrame = () => {
      const next = currentWindow();
      if (next === seenWindow) return;
      seenWindow = next;
      changed();
    };
    const onCommit = () => {
      seenWindow = currentWindow();
      changed();
    };
    const stops = [runtime.clock.subscribeFrame(onFrame), runtime.clock.subscribe(onCommit)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const held = useFrameWindows<ProductionSpendingReply>(runtime.client, identity, index => (productRequest({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'production-spending', params: { branchId: track!.branch.id, country: countryFromKey(tag!), repairs, window: index } })),
  () => wantedAt(runtime.clock.getSnapshot().position), subscribeClock, active, validate, track?.source);
  const needed = neededAt(runtime.clock.getSnapshot().position);
  const selected = useMemo(() => selectSpendingState(needed, held.data, held.errors, hour), [needed, held.data, held.errors, hour]);
  const data: SpendingData | undefined = selected.data;
  const error = selected.error;
  const retry = held.retry;
  return { track, tag, data, error, retry, window, hour, from, end, headAt };
}
