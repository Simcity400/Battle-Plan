import { useCallback, useEffect, useMemo, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, CountrySelection, WidgetSpec } from './model';
import type { Failure } from './queryError';
import type { Span } from '../charts/series';
import type { CountryIdentityHistory } from '../generated/dashboard';
import { defaultCountryKey, selectedCountryKeys } from './countrySelection.ts';
import { countryFromKey } from './countryKey.ts';
import { countryDrillTagAt, countryIdentityIndex } from './countryIdentity.ts';
import { runAt } from '../charts/series.ts';
import { useSlotTagsState } from './countrySlots.ts';
import { choiceAt, resolveChoice, type CountryHolder, type ReachedCountry } from './reachedCountry.ts';
import { boundCountryToken, datedCountrySpans, readDatedCountry, type BoundCountry, type DatedCountrySelection } from './datedCountry.ts';
import { worldFilterQuery, worldRequestTags } from './groups.ts';
import { useCountryGroups } from './useCountryGroups.ts';
import { axisAt, FOLLOW, recordedHour, refusedHours, regionFollows, regionHour, regionTrack, regionWindow, SPEC, validTime, type AxisHour, type BoundHour,
  type BoundWindow, type FollowQuery, type GatedHour, type GatedTime, type GatedWindow, type Outside, type PinTime, type Region, type RegionAction,
  type TrackHour } from './regionTime.ts';
import { epochOf, useRegion, useRegionHour, useRegionOutside } from './widgetTime.ts';

export interface CountryNeed { stored: unknown; key?: string; setOption?: (key: string, value: string) => void }
export interface GateNeeds { country?: CountryNeed; members?: boolean; holders?: boolean }

export interface CountryAt extends ReachedCountry {
  holders: CountryHolder[] | null; pick: (holder: CountryHolder) => void; error?: Failure; retry: () => void; historyHeld: boolean;
  aliasAt: (hour: number) => string | null; edges: readonly number[];
}
export interface MembersAt {
  countries: ReadonlySet<string> | null | undefined; requestTags: string[] | null | undefined; selectionReason: string | null; known: boolean;
  includes: (key: string) => boolean;
}
export interface Holder { tag: string; origin: number }
declare const HELD: unique symbol;
export type HeldCountry = { readonly tag: string; readonly origin: GatedHour; readonly slot: number; readonly countryId: number; readonly token: string }
  & { readonly [HELD]: 'held' };

export type GateStage =
  | { stage: 'unbound' }
  | { stage: 'outside'; outside: Outside }
  | { stage: 'choose' }
  | { stage: 'holders'; holders: CountryHolder[]; pick: (holder: CountryHolder) => void }
  | { stage: 'absent'; reason: string | null }
  | { stage: 'no-members'; reason: string }
  | { stage: 'failed'; failure: Failure; retry: () => void }
  | { stage: 'loading' }
  | { stage: 'open' };

export interface Gate {
  region: Region; track: BoundTrack | undefined; active: boolean;
  hour: GatedHour | null; window: GatedWindow | null; outside: Outside | null; stage: GateStage; open: boolean;
  country: CountryAt | null; members: MembersAt | null; groups: ReturnType<typeof useCountryGroups>;
  hourAt: (axis: AxisHour) => GatedHour | null;
  recorded: (hour: number | null | undefined, within?: readonly Span[] | null) => GatedHour | null;
  within: (hour: number | null | undefined) => BoundHour | null;
  span: (firstHour: number, lastHour: number) => BoundWindow | null;
  follow: (query: Readonly<Record<string, unknown>> | null) => FollowQuery | null;
  time: (window?: BoundWindow, mode?: 'pin' | 'range') => GatedTime | null;
  hold: (holder: Holder | null | undefined, extent?: BoundWindow | null) => HeldCountry | null;
}

const mint = (hour: TrackHour) => hour as GatedHour;
const bounded = (hour: number) => hour as BoundHour;
const bound = (selection: DatedCountrySelection) => selection as BoundCountry;

export function spanEdges(byTag: ReadonlyMap<string, readonly Span[]> | null | undefined): readonly number[] {
  const edges = new Set<number>();
  for (const spans of byTag?.values() ?? []) for (const span of spans) { edges.add(span.firstHour); edges.add(span.lastHour + 1); }
  return [...edges].sort((a, b) => a - b);
}

function slotEdges(histories: readonly CountryIdentityHistory[] | undefined, slot: number): readonly number[] {
  const edges = new Set<number>();
  for (const history of histories ?? []) for (const run of history.identities) if (run.slot === slot) { edges.add(run.firstHour); edges.add(run.lastHour + 1); }
  return [...edges].sort((a, b) => a - b);
}

const ranksCountries = (countries: CountrySelection) => countries.kind === 'all' || countries.kind === 'group' && countries.group.type === 'top';

const NO_EDGES: readonly number[] = [];

function mergedEdges(...lists: readonly (readonly number[])[]): readonly number[] {
  const held = lists.filter(list => list.length);
  if (held.length <= 1) return held[0] ?? NO_EDGES;
  return [...new Set(held.flat())].sort((x, y) => x - y);
}

const inside = (within: readonly Span[], hour: number) => within.some(span => span.firstHour <= hour && hour <= span.lastHour);

type Runtime = WidgetRuntime;

function useCountryAt(region: Region, runtime: Runtime, track: BoundTrack | undefined, groups: Gate['groups'], hour: TrackHour | null, on: boolean,
  need: CountryNeed | undefined): CountryAt | null {
  const widget = region.widget;
  const chosen = need ? selectedCountryKeys(widget.countries, track)[0] ?? null : null;
  const slot = chosen === null ? null : countryFromKey(chosen);
  const bySlot = typeof slot === 'number';
  const slotState = useSlotTagsState(runtime.client, track, on && bySlot && hour !== null);
  const slots = slotState.slots;
  const histories = groups.data?.countryIdentities;
  const campaign = track?.source.metadata.campaignKey, branch = track?.branch.id;
  const owner = JSON.stringify([campaign, branch, chosen]);
  const stored = need?.stored;
  const saved = useMemo(() => {
    const selection = campaign !== undefined && branch !== undefined ? readDatedCountry(stored, campaign, branch) : null;
    return selection?.tag === chosen ? selection : null;
  }, [stored, campaign, branch, chosen]);
  const [local, setLocal] = useState<{ owner: string; selection: DatedCountrySelection } | null>(null);
  const held = saved ?? (local?.owner === owner ? local.selection : null);
  const keep = (selection: DatedCountrySelection) => {
    setLocal({ owner, selection });
    need?.setOption?.(need.key ?? 'country', boundCountryToken(bound(selection)));
  };
  const fresh = need && !bySlot && !held && chosen && track && hour !== null && campaign !== undefined
    ? choiceAt(campaign, track.branch.id, track.branch.firstHour, chosen, hour) : null;
  const selection = bySlot ? null : held ?? fresh;
  const resolve = useCallback((choice: DatedCountrySelection) => histories && track && hour !== null
    ? resolveChoice(histories, groups.complete, choice, track.branch.firstHour, track.branch.lastHour, hour) : null, [histories, groups.complete, track, hour]);
  const resolved = useMemo(() => selection ? resolve(selection) : null, [selection, resolve]);
  const verified = useCallback((choice: DatedCountrySelection | null | undefined) => {
    const settled = choice ? resolve(choice) : null;
    return !!settled && settled.country.tag !== null && !settled.holders && !settled.rebind;
  }, [resolve]);
  const next = fresh && verified(fresh) ? fresh : resolved?.rebind && verified(resolved.rebind) ? resolved.rebind : null;
  const kept = next ? boundCountryToken(bound(next)) : null;
  useEffect(() => { if (next) keep(next); }, [kept]);
  const spans = useMemo(() => selection && histories && track ? datedCountrySpans(histories, selection, track.branch.firstHour, track.branch.lastHour) : null,
    [selection, histories, track]);
  const edges = useMemo(() => typeof slot === 'number' ? slotEdges(histories, slot) : spanEdges(spans), [histories, slot, spans]);
  if (!need) return null;
  const slotTag = bySlot && slots && hour !== null ? slots.tagAt(slot, hour) : null;
  const country: ReachedCountry = !chosen ? { chosen: null, tag: null, absent: false, reason: null }
    : bySlot ? { chosen, tag: slotTag, absent: !!slots && slotTag === null, reason: null }
    : resolved?.country ?? { chosen, tag: null, absent: false, reason: null };
  const error = bySlot ? slotState.error : groups.error;
  const aliasAt = (at: number) => bySlot ? slots?.tagAt(slot, at) ?? null : spans && !resolved?.holders && !resolved?.rebind ? countryDrillTagAt(spans, at) : null;
  return { ...country, holders: resolved?.holders ?? null,
    pick: holder => { if (campaign !== undefined && branch !== undefined && chosen && track)
      keep({ campaign, branch, tag: chosen, originOffset: holder.origin - track.branch.firstHour }); },
    error, retry: bySlot ? slotState.retry : groups.retry, historyHeld: bySlot ? !!slots : !!histories, aliasAt, edges };
}

export function useRegionGate(runtime: Runtime, region: Region, active: boolean, needs: GateNeeds = {}): Gate {
  const track = regionTrack(region, runtime.tracks);
  const widget = region.widget;
  const outside = useRegionOutside(runtime, region, active);
  const covered = !!track && outside === null;
  const wantsMembers = !!needs.members && !ranksCountries(widget.countries);
  const groups = useCountryGroups(runtime, covered && (!!needs.country || wantsMembers || !!needs.holders) ? track : undefined, active);
  const fallback = needs.members ? defaultCountryKey(track) : null;
  const lastHour = track?.branch.lastHour ?? null;
  const query = useMemo(() => wantsMembers && groups.data && lastHour !== null
    ? worldFilterQuery(widget.countries, fallback ? [fallback] : [], groups.data, lastHour) : null,
  [wantsMembers, widget.countries, fallback, groups.data, lastHour]);
  const request = useMemo(() => !needs.members || lastHour === null ? null
    : ranksCountries(widget.countries) ? worldRequestTags(widget.countries, [], undefined, lastHour)
    : groups.data ? worldRequestTags(widget.countries, fallback ? [fallback] : [], groups.data, lastHour) : null,
  [needs.members, widget.countries, fallback, groups.data, lastHour]);
  const histories = groups.data?.countryIdentities;
  const index = useMemo(() => wantsMembers && histories ? countryIdentityIndex(histories) : null, [wantsMembers, histories]);
  const chosenTags = wantsMembers && widget.countries.kind === 'tags' ? widget.countries.tags : null;
  const memberEdges = useMemo(() => chosenTags && histories ? spanEdges(new Map(histories.filter(history => chosenTags.includes(history.tag))
    .map(history => [history.tag, history.identities] as const))) : NO_EDGES, [chosenTags, histories]);
  const [countryEdges, setCountryEdges] = useState<readonly number[]>(NO_EDGES);
  const epochs = useMemo(() => mergedEdges(countryEdges, memberEdges, query?.edges ?? NO_EDGES), [countryEdges, memberEdges, query]);
  const hour = useRegionHour(runtime, region, active, epochs);
  const country = useCountryAt(region, runtime, track, groups, covered ? hour : null, active, needs.country);
  const edges = country?.edges ?? NO_EDGES;
  useEffect(() => { setCountryEdges(before => before.length === edges.length && before.every((edge, at) => edge === edges[at]) ? before : edges); }, [edges]);
  const members = useMemo<MembersAt | null>(() => {
    if (!needs.members) return null;
    if (!wantsMembers) return { countries: null, requestTags: request?.ready ? request.tags.length ? request.tags : null : undefined,
      selectionReason: null, known: true, includes: () => true };
    const filter = hour === null ? null : query?.at(hour) ?? null;
    const ids = filter?.countryIds;
    return { countries: filter && !filter.reason ? filter.tags : undefined,
      requestTags: request?.ready ? request.tags.length ? request.tags : null : undefined,
      selectionReason: filter ? filter.reason ?? request?.reason ?? null : null, known: filter?.known ?? false,
      includes: key => {
        if (!filter || filter.reason || hour === null) return false;
        if (filter.tags === null) return true;
        const id = index?.keyAt(key, hour) ?? null;
        return ids ? id !== null && ids.has(id) : filter.tags.has(key);
      } };
  }, [needs.members, wantsMembers, hour, query, request, index]);
  const stage = gateStage(track, outside, hour, country, needs.country !== undefined, wantsMembers, groups, members);
  const open = stage.stage === 'open';
  const gated = open && hour !== null ? mint(hour) : null;
  const tracks = runtime.tracks;
  const window = useMemo<GatedWindow | null>(() => {
    if (gated === null || !track) return null;
    const shown = regionWindow(region, track, tracks, axisAt(gated - track.axisOffset));
    return shown ? { firstHour: bounded(shown.firstHour), lastHour: gated } : null;
  }, [gated, region, track, tracks]);
  const follows = regionFollows(region);
  const hourAt = useCallback((axis: AxisHour) => {
    if (gated === null) return null;
    const at = regionHour(region, track, tracks, axis);
    return at !== null && epochOf(epochs, at) === epochOf(epochs, gated) ? mint(at) : null;
  }, [gated, region, track, tracks, epochs]);
  const aliasAt = country?.aliasAt;
  const recorded = useCallback((at: number | null | undefined, within?: readonly Span[] | null) => {
    if (gated === null) return null;
    const read = recordedHour(track, at);
    if (read === null || within && !inside(within, read) || aliasAt && aliasAt(read) === null) return null;
    return mint(read);
  }, [gated, track, aliasAt]);
  const within = useCallback((at: number | null | undefined) => gated === null || !track || at == null || !Number.isFinite(at) ? null
    : Math.floor(at) < track.branch.firstHour || Math.floor(at) > track.branch.lastHour ? null : bounded(Math.floor(at)), [gated, track]);
  const span = useCallback((firstHour: number, lastHour: number) => {
    if (gated === null || !track) return null;
    const first = Math.max(Math.floor(firstHour), track.branch.firstHour), last = Math.min(Math.floor(lastHour), track.branch.lastHour);
    return first <= last ? { firstHour: bounded(first), lastHour: bounded(last) } : null;
  }, [gated, track]);
  const follow = useCallback((query: Readonly<Record<string, unknown>> | null) =>
    gated !== null && follows && query && track && refusedHours(query, track.branch, true) === null ? query as FollowQuery : null, [gated, follows, track]);
  const time = useCallback((chosen?: BoundWindow, mode: 'pin' | 'range' = 'pin') => {
    if (!track || gated === null) return null;
    if (!chosen && follows) return validTime({ mode: 'follow' }, tracks);
    const held = chosen ?? { firstHour: gated, lastHour: gated };
    const first = track.branch.firstHour, offsets = { firstOffset: held.firstHour - first, lastOffset: held.lastHour - first };
    return validTime(mode === 'range' ? { mode, ...offsets } : { mode, windows: { [track.id]: offsets } }, tracks, track.id);
  }, [track, tracks, follows, gated]);
  const hold = useCallback((holder: Holder | null | undefined, extent?: BoundWindow | null) => {
    if (!track || gated === null || !holder || !histories) return null;
    const first = track.branch.firstHour, last = track.branch.lastHour, origin = recordedHour(track, holder.origin);
    if (origin === null) return null;
    const selection: DatedCountrySelection = { campaign: track.source.metadata.campaignKey, branch: track.branch.id, tag: holder.tag, originOffset: origin - first,
      ...extent ? { extent: [extent.firstHour - first, extent.lastHour - first] as [number, number] } : {} };
    const spans = datedCountrySpans(histories, selection, first, last);
    const identity = runAt(histories.find(history => history.tag === holder.tag)?.identities ?? [], origin);
    if (!spans || !identity || countryDrillTagAt(spans, gated) === null) return null;
    return { tag: holder.tag, origin: mint(origin), slot: identity.slot, countryId: identity.countryId, token: boundCountryToken(bound(selection)) } as HeldCountry;
  }, [track, gated, histories]);
  return { region, track, active, hour: gated, window, outside, stage, open, country, members, groups, hourAt, recorded, within, span, follow, time, hold };
}

function gateStage(track: BoundTrack | undefined, outside: Outside | null, hour: TrackHour | null, country: CountryAt | null, needsCountry: boolean,
  needsMembers: boolean, groups: Gate['groups'], members: MembersAt | null): GateStage {
  if (!track) return { stage: 'unbound' };
  if (outside) return { stage: 'outside', outside };
  if (hour === null) return { stage: 'loading' };
  if (needsCountry && country) {
    if (!country.chosen) return { stage: 'choose' };
    if (country.error && !country.historyHeld) return { stage: 'failed', failure: country.error, retry: country.retry };
    if (country.holders) return { stage: 'holders', holders: country.holders, pick: country.pick };
    if (country.reason) return { stage: 'absent', reason: country.reason };
    if (country.absent) return { stage: 'absent', reason: null };
    if (!country.tag) return { stage: 'loading' };
  }
  if (needsMembers && members) {
    if (!groups.data) return groups.error ? { stage: 'failed', failure: groups.error, retry: groups.retry } : { stage: 'loading' };
    if (members.selectionReason) return { stage: 'no-members', reason: members.selectionReason };
  }
  return { stage: 'open' };
}

export function pinNow(widget: WidgetSpec, tracks: readonly BoundTrack[], axis: AxisHour): GatedTime | null {
  const windows: Record<string, { firstOffset: number; lastOffset: number }> = {};
  for (const id of widget.trackIds) {
    const track = tracks.find(item => item.id === id);
    const window = track ? regionWindow({ widget, action: FOLLOW }, track, tracks, axis) : null;
    if (!track || !window || recordedHour(track, window.lastHour) === null) continue;
    windows[id] = { firstOffset: window.firstHour - track.branch.firstHour, lastOffset: window.lastHour - track.branch.firstHour };
  }
  return validTime({ mode: 'pin', windows } satisfies PinTime, tracks);
}

export function useCampaignGate(runtime: Runtime, widget: WidgetSpec, active = true): Gate {
  const campaign = useMemo(() => ({ ...widget, time: { mode: 'campaign' as const } }), [widget]);
  return useRegionGate(runtime, useRegion(campaign), active);
}

export function useGate(runtime: Runtime, widget: Region['widget'], time: RegionAction = SPEC, active = true, needs?: GateNeeds): Gate {
  return useRegionGate(runtime, useRegion(widget, time), active, needs);
}
