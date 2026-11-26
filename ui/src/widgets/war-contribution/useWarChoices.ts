import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import { dayLabel } from '../../charts/draw';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { resolveWindow, type WidgetBase, type WidgetSpec } from '../../dashboard/model';
import { entryAt } from '../../dashboard/reads.ts';
import { setToken } from '../../dashboard/options';
import { type ProductState, useProduct } from '../../dashboard/products';
import type { WarsReply } from '../../generated/dashboard';
import { validateWarsReply } from '../wars/warsData';
import { useHeadHour } from '../army-size/headHour';
import { warChoicesAt } from './warContributionData';
import { savedWar } from './warContributionPlan';
import { productRequest } from '../../dashboard/request.ts';

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
  const starts = useMemo(() => (reply.data?.wars ?? []).flatMap(war => war.observations).sort((a, b) => a.firstHour - b.firstHour), [reply.data]);
  const date = (clock: number) => track ? resolveWindow(widget, track, runtime.tracks, clock)?.lastHour ?? -Infinity : -Infinity;
  const edge = (clock: number) => entryAt(starts, date(clock))?.firstHour ?? -Infinity;
  const clock = useHeadHour(runtime.clock, !!reply.data && widget.time.mode === 'follow', hour => String(edge(hour)));
  const at = edge(clock);
  const choices = useMemo(() => warChoicesAt(reply.data, at, saved), [reply.data, at, saved]);
  return { reply, choices };
}

export function useWarChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { choices } = useWarMenu(widget, runtime, savedWar(widget.options ?? {}));
  return useMemo(() => ({ war: choices.map(war => ({ value: setToken(war.id),
    label: war.firstHour === null ? war.name : `${war.name} · ${dayLabel(war.firstHour)}` })) }), [choices]);
}
