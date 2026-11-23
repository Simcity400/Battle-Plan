import { useCallback, useEffect, useMemo, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, CountrySelection, WidgetSpec } from './model';
import type { Failure } from './queryError';
import type { Span } from '../charts/series';
import type { CountryIdentityHistory } from '../generated/dashboard';
import { defaultCountryKey, selectedCountryKeys } from './countrySelection.ts';
import { countryFromKey, countryKey } from './countryKey.ts';
import { countryDrillTagAt, countryIdentityIndex } from './countryIdentity.ts';
import { useSlotTagsState } from './countrySlots.ts';
import { choiceAt, resolveChoice, type CountryHolder, type ReachedCountry } from './reachedCountry.ts';
import { boundCountryToken, datedCountrySpans, readDatedCountry, type BoundCountry, type DatedCountrySelection } from './datedCountry.ts';
import { selectionChangeHours, worldFilterQuery, worldRequestTags } from './groups.ts';
import { useCountryGroups } from './useCountryGroups.ts';
import { axisAt, checkedTime, followedProduct, FOLLOW, recordedHour, refusedHours, regionFollows, regionHour, regionTrack, regionWindow, SPEC, type AxisHour,
  type BoundHour, type BoundWindow, type FollowQuery, type GatedHour, type GatedTime, type GatedWindow, type Outside, type PinTime, type Region, type RegionAction,
  type TimeRefusal, type TrackHour } from './regionTime.ts';
import { epochOf, useRegion, useRegionHour, useRegionOutside } from './widgetTime.ts';
import { NO_MEMBERS } from './noMembers.ts';

export interface CountryNeed { stored?: unknown; held?: Holder | null; key?: string; setOption?: (key: string, value: string) => void }
export interface GateNeeds { country?: CountryNeed; members?: boolean; holders?: boolean; changes?: readonly number[] }

export interface CountryAt extends ReachedCountry {
  holders: CountryHolder[] | null; pick: (holder: CountryHolder) => void; error?: Failure; retry: () => void; historyHeld: boolean;
  aliasAt: (hour: number) => string | null; edges: readonly number[]; holder: Holder | null;
  history: { key: string; spans: readonly Span[] } | null; aliases: ReadonlyMap<string, readonly Span[]> | null;
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
  country: CountryAt | null; members: MembersAt | null; groups: ReturnType<typeof useCountryGroups>; held: HeldCountry | null;
  hourAt: (axis: AxisHour) => GatedHour | null;
  recorded: (hour: number | null | undefined, within?: readonly Span[] | null) => GatedHour | null;
  within: (hour: number | null | undefined) => BoundHour | null;
  span: (firstHour: number, lastHour: number) => BoundWindow | null;
  follow: (query: Readonly<Record<string, unknown>> | null) => FollowQuery | null;
  checkFollow: (query: Readonly<Record<string, unknown>> | null) => FollowRefusal;
  time: (window?: BoundWindow, mode?: 'pin' | 'range') => GatedTime | null;
  checkTime: (window?: BoundWindow, mode?: 'pin' | 'range') => TimeRefusal;
  hold: (holder: Holder | null | undefined, extent?: BoundWindow | null) => HeldCountry | null;
  checkHold: (holder: Holder | null | undefined, extent?: BoundWindow | null) => HoldRefusal;
}

export type FollowRefusal = { query: FollowQuery; refused: null } | { query: null; refused: string | null };
const NOT_FOLLOWED: FollowRefusal = { query: null, refused: null };
export type HoldRefusal = { held: HeldCountry; refused?: undefined } | { held: null; refused: string };
const NOTHING_HERE = 'This time is not recorded, so there is nothing to pin.';
const NOT_HERE: TimeRefusal = { time: null, refused: NOTHING_HERE };

const mint = (hour: TrackHour) => hour as GatedHour;
const bounded = (hour: number) => hour as BoundHour;
const bound = (selection: DatedCountrySelection) => selection as BoundCountry;

export function spanEdges(byTag: ReadonlyMap<string, readonly Span[]> | null | undefined): readonly number[] {
  const edges = new Set<number>();
  for (const spans of byTag?.values() ?? []) for (const span of spans) { edges.add(span.firstHour); edges.add(span.lastHour + 1); }
  return [...edges].sort((a, b) => a - b);
}

export function useHeldEdges(): [readonly number[], (spans: ReadonlyMap<string, readonly Span[]> | null | undefined) => void] {
  const [edges, setEdges] = useState<readonly number[]>(NO_EDGES);
  const hold = useCallback((spans: ReadonlyMap<string, readonly Span[]> | null | undefined) => {
    const next = spanEdges(spans);
    setEdges(before => before.length === next.length && before.every((edge, at) => edge === next[at]) ? before : next);
  }, []);
  return [edges, hold];
}

function runEdges(runs: readonly Span[]): readonly number[] {
  const edges = new Set<number>();
  for (const run of runs) { edges.add(run.firstHour); edges.add(run.lastHour + 1); }
  return [...edges].sort((a, b) => a - b);
}

export function holderSelection(histories: readonly CountryIdentityHistory[], track: Pick<BoundTrack, 'source' | 'branch'>, holder: Holder): DatedCountrySelection | null {
  const origin = Math.floor(holder.origin), first = track.branch.firstHour;
  if (!Number.isSafeInteger(origin) || origin < first || origin > track.branch.lastHour) return null;
  const index = countryIdentityIndex(histories), key = index.keyAt(holder.tag, origin);
  if (key === null) return null;
  const aliases = index.aliasesAt(new Set([key]), origin);
  const tag = aliases.includes(holder.tag) ? holder.tag : aliases.length === 1 ? aliases[0] : null;
  return tag === null ? null : { campaign: track.source.metadata.campaignKey, branch: track.branch.id, tag, originOffset: origin - first };
}

function identityHistory(histories: readonly CountryIdentityHistory[], selection: DatedCountrySelection, first: number, last: number) {
  const index = countryIdentityIndex(histories), key = index.keyAt(selection.tag, first + selection.originOffset);
  if (key === null) return null;
  const identity = index.identityOf(key);
  const spans: Span[] = [];
  for (const run of index.aliasRuns(new Set([key]), first, last)) {
    const previous = spans.at(-1);
    if (previous && run.firstHour <= previous.lastHour + 1) previous.lastHour = Math.max(previous.lastHour, run.lastHour);
    else spans.push({ firstHour: run.firstHour, lastHour: run.lastHour });
  }
  return { ...identity, history: { key: countryKey(identity.slot), spans } };
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
  const picked = need?.held !== undefined;
  const pick = need?.held ?? null;
  const chosen = !need ? null : picked ? pick?.tag ?? null : selectedCountryKeys(widget.countries, track)[0] ?? null;
  const slot = chosen === null || picked ? null : countryFromKey(chosen);
  const bySlot = typeof slot === 'number';
  const slotState = useSlotTagsState(runtime.client, track, on && bySlot && hour !== null);
  const slots = slotState.slots;
  const histories = groups.data?.countryIdentities;
  const campaign = track?.source.metadata.campaignKey, branch = track?.branch.id;
  const owner = JSON.stringify([campaign, branch, chosen]);
  const stored = need?.stored;
  const saved = useMemo(() => {
    const selection = !picked && campaign !== undefined && branch !== undefined ? readDatedCountry(stored, campaign, branch) : null;
    return selection?.tag === chosen ? selection : null;
  }, [picked, stored, campaign, branch, chosen]);
  const pickedSelection = useMemo(() => pick && histories && track ? holderSelection(histories, track, pick) : null,
    [pick?.tag, pick?.origin, histories, track]);
  const [local, setLocal] = useState<{ owner: string; selection: DatedCountrySelection } | null>(null);
  const held = saved ?? (local?.owner === owner ? local.selection : null);
  const keep = (selection: DatedCountrySelection) => {
    setLocal({ owner, selection });
    need?.setOption?.(need.key ?? 'country', boundCountryToken(bound(selection)));
  };
  const fresh = need && !picked && !bySlot && !held && chosen && track && hour !== null && campaign !== undefined
    ? choiceAt(campaign, track.branch.id, track.branch.firstHour, chosen, hour) : null;
  const selection = picked ? pickedSelection : bySlot ? null : held ?? fresh;
  const resolve = useCallback((choice: DatedCountrySelection) => histories && track && hour !== null
    ? resolveChoice(histories, groups.complete, choice, track.branch.firstHour, track.branch.lastHour, hour) : null, [histories, groups.complete, track, hour]);
  const resolved = useMemo(() => selection ? resolve(selection) : null, [selection, resolve]);
  const verified = useCallback((choice: DatedCountrySelection | null | undefined) => {
    const settled = choice ? resolve(choice) : null;
    return !!settled && settled.country.tag !== null && !settled.holders && !settled.rebind;
  }, [resolve]);
  const next = picked ? null : fresh && verified(fresh) ? fresh : resolved?.rebind && verified(resolved.rebind) ? resolved.rebind : null;
  const kept = next ? boundCountryToken(bound(next)) : null;
  useEffect(() => { if (next) keep(next); }, [kept]);
  const spans = useMemo(() => selection && histories && track ? datedCountrySpans(histories, selection, track.branch.firstHour, track.branch.lastHour) : null,
    [selection, histories, track]);
  const slotRuns = useMemo(() => bySlot && slots ? slots.tagRuns(slot) : null, [bySlot, slots, slot]);
  const identity = useMemo(() => selection && histories && track && !resolved?.holders && !resolved?.rebind
    ? identityHistory(histories, selection, track.branch.firstHour, track.branch.lastHour) : null, [selection, histories, track, resolved?.holders, resolved?.rebind]);
  const edges = useMemo(() => slotRuns ? runEdges(slotRuns) : spanEdges(spans), [slotRuns, spans]);
  const slotAliases = useMemo(() => {
    if (!slotRuns) return null;
    const byTag = new Map<string, Span[]>();
    for (const run of slotRuns) { const spansOf = byTag.get(run.tag); const span = { firstHour: run.firstHour, lastHour: run.lastHour }; if (spansOf) spansOf.push(span); else byTag.set(run.tag, [span]); }
    return byTag;
  }, [slotRuns]);
  if (!need) return null;
  const slotTag = bySlot && slots && hour !== null ? slots.tagAt(slot, hour) : null;
  const unresolved = picked && !!pick && !!histories && groups.complete && !pickedSelection;
  const country: ReachedCountry = !chosen ? { chosen: null, tag: null, absent: false, reason: null }
    : unresolved ? { chosen, tag: null, absent: false, reason: 'The chosen country’s recorded identity is unavailable.' }
    : bySlot ? { chosen, tag: slotTag, absent: !!slots && slotTag === null, reason: null }
    : resolved?.country ?? { chosen, tag: null, absent: false, reason: null };
  const error = bySlot ? slotState.error : groups.error;
  const aliasAt = (at: number) => bySlot ? slots?.tagAt(slot, at) ?? null : spans && !resolved?.holders && !resolved?.rebind ? countryDrillTagAt(spans, at) : null;
  const holder: Holder | null = bySlot ? chosen && hour !== null && slotTag ? { tag: chosen, origin: hour } : null
    : selection && track && country.tag ? { tag: selection.tag, origin: track.branch.firstHour + selection.originOffset } : null;
  const history = bySlot ? slotRuns && chosen ? { key: chosen, spans: slotRuns.map(({ firstHour, lastHour }) => ({ firstHour, lastHour })) } : null
    : identity?.history ?? null;
  return { ...country, holders: resolved?.holders ?? null,
    pick: holder => { if (!picked && campaign !== undefined && branch !== undefined && chosen && track)
      keep({ campaign, branch, tag: chosen, originOffset: holder.origin - track.branch.firstHour }); },
    error, retry: bySlot ? slotState.retry : groups.retry, historyHeld: bySlot ? !!slots : !!histories && groups.complete, aliasAt, edges, holder, history,
    aliases: bySlot ? slotAliases : resolved?.holders || resolved?.rebind ? null : spans };
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
  const memberEdges = useMemo(() => wantsMembers ? selectionChangeHours(widget.countries, fallback ? [fallback] : [], groups.data, index) : NO_EDGES,
    [wantsMembers, widget.countries, fallback, groups.data, index]);
  const [countryEdges, setCountryEdges] = useState<readonly number[]>(NO_EDGES);
  const changes = needs.changes ?? NO_EDGES;
  const epochs = useMemo(() => mergedEdges(countryEdges, memberEdges, changes), [countryEdges, memberEdges, changes]);
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
    return shown ? { firstHour: bounded(Math.min(gated, Math.max(track.branch.firstHour, Math.floor(shown.firstHour)))), lastHour: gated } : null;
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
  const checkFollow = useCallback((query: Readonly<Record<string, unknown>> | null): FollowRefusal => {
    if (gated === null || !follows || !query || !track) return NOT_FOLLOWED;
    const asked = followedProduct(query);
    if (!asked) return { query: null, refused: 'This reading names no product to follow.' };
    const refused = refusedHours(asked.product, asked.params, track.branch, true);
    return refused === null ? { query: query as FollowQuery, refused: null } : { query: null, refused };
  }, [gated, follows, track]);
  const follow = useCallback((query: Readonly<Record<string, unknown>> | null) => checkFollow(query).query, [checkFollow]);
  const checkTime = useCallback((chosen?: BoundWindow, mode: 'pin' | 'range' = 'pin'): TimeRefusal => {
    if (!track || gated === null) return NOT_HERE;
    if (!chosen && follows) return checkedTime({ mode: 'follow' }, tracks);
    const held = chosen ?? { firstHour: gated, lastHour: gated };
    const first = track.branch.firstHour, offsets = { firstOffset: held.firstHour - first, lastOffset: held.lastHour - first };
    return checkedTime(mode === 'range' ? { mode, ...offsets } : { mode, windows: { [track.id]: offsets } }, tracks, track.id);
  }, [track, tracks, follows, gated]);
  const time = useCallback((chosen?: BoundWindow, mode: 'pin' | 'range' = 'pin') => checkTime(chosen, mode).time, [checkTime]);
  const identitiesError = groups.error;
  const checkHold = useCallback((holder: Holder | null | undefined, extent?: BoundWindow | null): HoldRefusal => {
    if (!track || gated === null) return { held: null, refused: NOTHING_HERE };
    if (!holder) return { held: null, refused: 'No country is chosen for this moment.' };
    if (!histories) return { held: null, refused: identitiesError ? 'Country identities could not be loaded, so the country cannot be pinned.' : 'Country identities are still loading.' };
    if (recordedHour(track, holder.origin) === null) return { held: null, refused: 'The hour the country was picked was not recorded.' };
    const first = track.branch.firstHour, last = track.branch.lastHour, picked = holderSelection(histories, track, holder);
    if (!picked) return { held: null, refused: 'The picked country’s recorded identity is unavailable.' };
    const selection: DatedCountrySelection = { ...picked, ...extent ? { extent: [extent.firstHour - first, extent.lastHour - first] as [number, number] } : {} };
    const spans = datedCountrySpans(histories, selection, first, last);
    const identity = identityHistory(histories, selection, first, last);
    if (!spans || !identity || countryDrillTagAt(spans, gated) === null) return { held: null, refused: 'The picked country holds no tag at this time.' };
    return { held: { tag: selection.tag, origin: mint(first + selection.originOffset as TrackHour), slot: identity.slot, countryId: identity.countryId,
      token: boundCountryToken(bound(selection)) } as HeldCountry };
  }, [track, gated, histories, identitiesError]);
  const hold = useCallback((holder: Holder | null | undefined, extent?: BoundWindow | null) => checkHold(holder, extent).held, [checkHold]);
  const holder = country?.holder ?? null;
  const held = useMemo(() => hold(holder), [hold, holder?.tag, holder?.origin]);
  return { region, track, active, hour: gated, window, outside, stage, open, country, members, groups, held, hourAt, recorded, within, span, follow, checkFollow, time, checkTime, hold, checkHold };
}

export function gateStage(track: BoundTrack | undefined, outside: Outside | null, hour: TrackHour | null, country: CountryAt | null, needsCountry: boolean,
  needsMembers: boolean, groups: Gate['groups'], members: MembersAt | null): GateStage {
  if (!track) return { stage: 'unbound' };
  if (outside) return { stage: 'outside', outside };
  if (hour === null) return { stage: 'loading' };
  if (needsCountry && country) {
    if (!country.chosen) return { stage: 'choose' };
    if (country.error && !country.historyHeld) return { stage: 'failed', failure: country.error, retry: country.retry };
    if (country.holders) return { stage: 'holders', holders: country.holders, pick: country.pick };
    if (country.reason) return { stage: 'absent', reason: country.reason };
    if (country.absent || country.aliases?.size === 0) return { stage: 'absent', reason: null };
    if (!country.tag) return country.historyHeld ? { stage: 'absent', reason: null } : { stage: 'loading' };
  }
  if (needsMembers && members) {
    if (!groups.data) return groups.error ? { stage: 'failed', failure: groups.error, retry: groups.retry } : { stage: 'loading' };
    if (members.selectionReason) return { stage: 'no-members', reason: members.selectionReason };
    if (members.countries?.size === 0) return { stage: 'no-members', reason: NO_MEMBERS.hour };
  }
  return { stage: 'open' };
}

export function pinNow(widget: WidgetSpec, tracks: readonly BoundTrack[], axis: AxisHour): TimeRefusal {
  const windows: Record<string, { firstOffset: number; lastOffset: number }> = {};
  for (const id of widget.trackIds) {
    const track = tracks.find(item => item.id === id);
    const window = track ? regionWindow({ widget, action: FOLLOW }, track, tracks, axis) : null;
    if (!track || !window) continue;
    windows[id] = { firstOffset: Math.floor(window.firstHour) - track.branch.firstHour, lastOffset: Math.floor(window.lastHour) - track.branch.firstHour };
  }
  return checkedTime({ mode: 'pin', windows } satisfies PinTime, tracks);
}

export function useCampaignGate(runtime: Runtime, widget: WidgetSpec, active = true): Gate {
  const campaign = useMemo(() => ({ ...widget, time: { mode: 'campaign' as const } }), [widget]);
  return useRegionGate(runtime, useRegion(campaign), active);
}

export function useGate(runtime: Runtime, widget: Region['widget'], time: RegionAction = SPEC, active = true, needs?: GateNeeds): Gate {
  return useRegionGate(runtime, useRegion(widget, time), active, needs);
}
