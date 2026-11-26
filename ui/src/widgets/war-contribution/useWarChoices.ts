import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useSyncExternalStore } from 'react';
import { dayLabel } from '../../charts/draw';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { resolveWindow, type WidgetBase, type WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { type ProductState, useProduct } from '../../dashboard/products';
import { watchBoundary } from '../../dashboard/widgetTime';
import type { WarsReply } from '../../generated/dashboard';
import { validateWarsReply } from '../wars/warsData';
import { createWarMenuTracker, warChoicesAt } from './warContributionData';
import { savedWar } from './warContributionPlan';
import { productRequest } from '../../dashboard/request.ts';

const NO_SUBSCRIBE = () => () => {};

export function useWarsReply(widget: WidgetBase, runtime: WidgetRuntime, active: boolean): ProductState<WarsReply> {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const viewpointTag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  return useProduct<WarsReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'wars',
    params: { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) } }) : null, active, validateWarsReply, track?.source);
}

export function useWars(widget: WidgetBase, runtime: WidgetRuntime, active: boolean): WarsReply | undefined {
  return useWarsReply(widget, runtime, active).data;
}

export function useWarMenu(widget: WidgetBase, runtime: WidgetRuntime, saved: string | null) {
  const reply = useWarsReply(widget, runtime, true);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const shownHour = useCallback(
    (clockHour: number) => (track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null),
    [track, widget, runtime.tracks],
  );
  const offset = track?.axisOffset ?? 0;
  const tracker = useMemo(
    () => createWarMenuTracker(reply.data, shownHour, offset),
    [reply.data, shownHour, offset],
  );
  const follow = !!reply.data && !!track && widget.time.mode === 'follow';
  const subscribe = useCallback(
    (changed: () => void) => (follow ? watchBoundary(runtime.clock, tracker.span, changed) : NO_SUBSCRIBE()),
    [follow, runtime.clock, tracker],
  );
  const read = useCallback(() => tracker.read(runtime.clock.getHour()), [tracker, runtime.clock]);
  const at = useSyncExternalStore(subscribe, read, read);
  const choices = useMemo(() => warChoicesAt(reply.data, at, saved), [reply.data, at, saved]);
  return { reply, choices };
}

export function useWarChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { choices } = useWarMenu(widget, runtime, savedWar(widget.options ?? {}));
  return useMemo(() => ({ war: choices.map(war => ({ value: setToken(war.id),
    label: war.firstHour === null ? war.name : `${war.name} · ${dayLabel(war.firstHour)}` })) }), [choices]);
}
