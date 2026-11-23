import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useSlotTags } from '../../dashboard/countrySlots.ts';
import { useMemo } from 'react';
import type { OptionChoices, WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { resolveCountries } from '../../dashboard/groups';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useProduct, useFollowedProduct } from '../../dashboard/products';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import type { ApmParams, ApmReply } from '../../generated/dashboard';
import { GRAPH_OPTIONS, TABLE_APM_OPTIONS } from './definition';
import { playerNames, validateApm } from './apmData';

export function useApmRegistry(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'player-apm',
    params: { branchId: track.branch.id, group: 'player', registry: true, firstHour: null, lastHour: null, tags: null, membership: null, players: [], buckets: 1 } } : null;
  return useProduct<ApmReply>(runtime.client, request, active, validateApm, track?.source);
}

export function useApmChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useApmRegistry(widget, runtime, true);
  return useMemo(() => {
    const players = data?.players ?? [];
    const names = playerNames(players);
    return { players: players.map(player => ({ value: setToken(player.id), label: `${names.get(player.id)} · ${player.tags.join(', ')}` })) };
  }, [data]);
}

export function useApm({ widget, runtime, active }: WidgetProps, graph: boolean, view?: { identity: string; from: number; to: number; width: number } | null,
  followed = false) {
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const identity = JSON.stringify([track?.source.id, track?.branch.id, widget.time]);
  const options = resolveOptions(graph ? GRAPH_OPTIONS : TABLE_APM_OPTIONS, widget.options);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const rangeFirst = window && Math.max(track!.branch.firstHour, Math.floor(window.firstHour));
  const rangeLast = window && Math.min(track!.branch.lastHour, Math.floor(window.lastHour));
  const first = graph && widget.time.mode === 'follow' ? track?.branch.firstHour : rangeFirst;
  const last = graph && widget.time.mode === 'follow' ? track?.branch.lastHour : rangeLast;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const selected = selectedCountryKeys(widget.countries, track);
  const resolved = useMemo(() => widget.countries.kind === 'all' ? null
    : resolveCountries(widget.countries, selected, groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
  [widget.countries, JSON.stringify(selected), groups.data, track?.branch.lastHour]);
  const slot = widget.countries.kind === 'selected' && selected.length === 1 ? countryFromKey(selected[0]) : null;
  const slots = useSlotTags(runtime.client, track, active && typeof slot === 'number');
  const asked = useMemo(() => {
    if (typeof slot !== 'number' || !resolved) return resolved;
    const runs = slots?.tagRuns(slot) ?? [];
    return { ...resolved, tags: [...new Set(runs.map(run => run.tag))], membership: new Map([['', runs]]),
      known: (from: number, to: number) => !!slots && resolved.known(from, to) };
  }, [resolved, slot, slots]);
  const boundedView = graph && view?.identity === identity && first != null && last != null && view.from <= last && view.to >= first ? view : null;
  const firstHour = first == null || rangeFirst == null ? null : Math.max(first, boundedView ? Math.floor(boundedView.from) : rangeFirst);
  const lastHour = last == null || rangeLast == null ? null : Math.min(last, boundedView ? Math.floor(boundedView.to) : rangeLast);
  const known = firstHour !== null && lastHour !== null && (asked?.known(firstHour, lastHour) ?? true);
  const membershipGap = resolved?.unobserved.some(span => firstHour !== null && lastHour !== null && span.firstHour <= lastHour && firstHour <= span.lastHour);
  const reason = resolved?.reason ?? (membershipGap ? 'Country group membership was not recorded throughout this period.' : null);
  const params: ApmParams | null = !followed && track && firstHour !== null && lastHour !== null && firstHour <= lastHour && known && !reason ? {
    branchId: track.branch.id, group: options.group === 'player' ? 'player' : 'country', registry: false, firstHour, lastHour,
    tags: asked?.tags ?? null, players: contentSet(options.players).map(contentKey),
    membership: asked?.membership ? [...asked.membership.values()].flat() : null,
    buckets: graph ? Math.max(1, Math.min(4096, Math.ceil(view?.width ?? 1024))) : 1,
  } : null;
  const product = useFollowedProduct<ApmReply>(runtime.client, track && params ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'player-apm', params } : null, ['firstHour', 'lastHour', 'buckets'], active, validateApm, track?.source);
  const answered = useMemo(() => {
    if (!product.moment) return null;
    const [firstHour, lastHour] = JSON.parse(product.moment) as [number, number, number];
    return { firstHour, lastHour };
  }, [product.moment]);
  const head = track && (!resolved || (!resolved.reason && !membershipGap)) ? {
    branchId: track.branch.id, group: options.group === 'player' ? 'player' as const : 'country' as const, tags: asked?.tags ?? null,
    membership: asked?.membership ? [...asked.membership.values()].flat() : null, players: contentSet(options.players).map(contentKey),
  } : null;
  return { track, identity, options, window, first, last, playbackHour, known, reason, groups, answered, head, ...product };
}
