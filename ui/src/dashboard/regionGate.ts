import { useCallback, useEffect, useMemo, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, CountrySelection, WidgetSpec } from './model';
import type { Failure } from './queryError';
import type { CountryIdentityHistory } from '../generated/dashboard';
import { defaultCountryKey, selectedCountryKeys } from './countrySelection.ts';
import { countryFromKey } from './countryKey.ts';
import { countryIdentityIndex } from './countryIdentity.ts';
import { useSlotTagsState } from './countrySlots.ts';
import { choiceAt, resolveChoice, type CountryHolder, type ReachedCountry } from './reachedCountry.ts';
import { boundCountryToken, readDatedCountry, type BoundCountry, type DatedCountrySelection } from './datedCountry.ts';
import { worldFilterQuery, worldRequestTags } from './groups.ts';
import { useCountryGroups } from './useCountryGroups.ts';
import { axisAt, FOLLOW, recordedHour, regionFollows, regionHour, regionTrack, regionWindow, SPEC, type AxisHour, type FollowQuery, type GatedHour,
  type GatedTime, type GatedWindow, type Outside, type Region, type RegionAction, type TrackHour } from './regionTime.ts';
import { epochOf, useRegion, useRegionHour, useRegionOutside } from './widgetTime.ts';

export interface CountryNeed { stored: unknown; setOption?: (key: string, value: string) => void }
export interface GateNeeds { country?: CountryNeed; members?: boolean }

export interface CountryAt extends ReachedCountry {
  holders: CountryHolder[] | null; pick: (holder: CountryHolder) => void; error?: Failure; retry: () => void; historyHeld: boolean;
}
export interface MembersAt {
  countries: ReadonlySet<string> | null | undefined; requestTags: string[] | null | undefined; selectionReason: string | null; known: boolean;
  includes: (key: string) => boolean;
}

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
  recorded: (hour: number | null | undefined) => GatedHour | null;
  within: (hour: number | null | undefined) => GatedHour | null;
  span: (firstHour: number, lastHour: number) => GatedWindow | null;
  follow: (query: Readonly<Record<string, unknown>> | null) => FollowQuery | null;
  time: (window?: GatedWindow, mode?: 'pin' | 'range') => GatedTime | null;
  bind: (tag: string, extent?: GatedWindow) => string | null;
}

const mint = (hour: TrackHour) => hour as GatedHour;
const bound = (selection: DatedCountrySelection) => selection as BoundCountry;
type Time = { mode: 'follow' } | { mode: 'pin'; windows: Record<string, { firstOffset: number; lastOffset: number }> } | { mode: 'range'; firstOffset: number; lastOffset: number };
const mintTime = (time: Time) => time as GatedTime;

export function identityEpochs(histories: readonly CountryIdentityHistory[] | undefined): readonly number[] {
  const edges = new Set<number>();
  for (const history of histories ?? []) for (const run of history.identities) { edges.add(run.firstHour); edges.add(run.lastHour + 1); }
  return [...edges].sort((a, b) => a - b);
}

const ranksCountries = (countries: CountrySelection) => countries.kind === 'all' || countries.kind === 'group' && countries.group.type === 'top';

const NO_EDGES: readonly number[] = [];

function mergedEdges(a: readonly number[], b: readonly number[]): readonly number[] {
  if (!b.length) return a;
  if (!a.length) return b;
  return [...new Set([...a, ...b])].sort((x, y) => x - y);
}

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
    need?.setOption?.('country', boundCountryToken(bound(selection)));
  };
  const fresh = need && !bySlot && !held && chosen && track && hour !== null && campaign !== undefined
    ? choiceAt(campaign, track.branch.id, track.branch.firstHour, chosen, hour) : null;
  const selection = bySlot ? null : held ?? fresh;
  const resolved = useMemo(() => selection && histories && track && hour !== null
    ? resolveChoice(histories, groups.complete, selection, track.branch.firstHour, track.branch.lastHour, hour) : null,
  [selection, histories, groups.complete, track, hour]);
  const kept = fresh ? boundCountryToken(bound(fresh)) : resolved?.rebind ? boundCountryToken(bound(resolved.rebind)) : null;
  useEffect(() => {
    const next = fresh ?? resolved?.rebind;
    if (next) keep(next);
  }, [kept]);
  if (!need) return null;
  const slotTag = bySlot && slots && hour !== null ? slots.tagAt(slot, hour) : null;
  const country: ReachedCountry = !chosen ? { chosen: null, tag: null, absent: false, reason: null }
    : bySlot ? { chosen, tag: slotTag, absent: !!slots && slotTag === null, reason: null }
    : resolved?.country ?? { chosen, tag: null, absent: false, reason: null };
  const error = bySlot ? slotState.error : groups.error;
  return { ...country, holders: resolved?.holders ?? null,
    pick: holder => { if (campaign !== undefined && branch !== undefined && chosen && track)
      keep({ campaign, branch, tag: chosen, originOffset: holder.origin - track.branch.firstHour }); },
    error, retry: bySlot ? slotState.retry : groups.retry, historyHeld: bySlot ? !!slots : !!histories };
}

export function useRegionGate(runtime: Runtime, region: Region, active: boolean, needs: GateNeeds = {}): Gate {
  const track = regionTrack(region, runtime.tracks);
  const widget = region.widget;
  const outside = useRegionOutside(runtime, region, active);
  const covered = !!track && outside === null;
  const wantsMembers = !!needs.members && !ranksCountries(widget.countries);
  const groups = useCountryGroups(runtime, covered && (!!needs.country || wantsMembers) ? track : undefined, active);
  const fallback = needs.members ? defaultCountryKey(track) : null;
  const lastHour = track?.branch.lastHour ?? null;
  const query = useMemo(() => wantsMembers && groups.data && lastHour !== null
    ? worldFilterQuery(widget.countries, fallback ? [fallback] : [], groups.data, lastHour) : null,
  [wantsMembers, widget.countries, fallback, groups.data, lastHour]);
  const request = useMemo(() => !needs.members || lastHour === null ? null
    : ranksCountries(widget.countries) ? worldRequestTags(widget.countries, [], undefined, lastHour)
    : groups.data ? worldRequestTags(widget.countries, fallback ? [fallback] : [], groups.data, lastHour) : null,
  [needs.members, widget.countries, fallback, groups.data, lastHour]);
  const histories = needs.country || wantsMembers ? groups.data?.countryIdentities : undefined;
  const index = useMemo(() => histories ? countryIdentityIndex(histories) : null, [histories]);
  const identity = useMemo(() => histories ? identityEpochs(histories) : NO_EDGES, [histories]);
  const epochs = useMemo(() => mergedEdges(identity, query?.edges ?? NO_EDGES), [identity, query]);
  const hour = useRegionHour(runtime, region, active, epochs);
  const country = useCountryAt(region, runtime, track, groups, covered ? hour : null, active, needs.country);
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
    return shown ? { firstHour: mint(shown.firstHour as TrackHour), lastHour: gated } : null;
  }, [gated, region, track, tracks]);
  const follows = regionFollows(region);
  const hourAt = useCallback((axis: AxisHour) => {
    if (gated === null) return null;
    const at = regionHour(region, track, tracks, axis);
    return at !== null && epochOf(epochs, at) === epochOf(epochs, gated) ? mint(at) : null;
  }, [gated, region, track, tracks, epochs]);
  const recorded = useCallback((at: number | null | undefined) => gated === null ? null : recordedHour(track, at) === null ? null : mint(Math.floor(at!) as TrackHour),
    [gated, track]);
  const within = useCallback((at: number | null | undefined) => gated === null || !track || at == null || !Number.isFinite(at) ? null
    : Math.floor(at) < track.branch.firstHour || Math.floor(at) > track.branch.lastHour ? null : mint(Math.floor(at) as TrackHour), [gated, track]);
  const span = useCallback((firstHour: number, lastHour: number) => {
    if (gated === null || !track) return null;
    const first = Math.max(Math.floor(firstHour), track.branch.firstHour), last = Math.min(Math.floor(lastHour), track.branch.lastHour);
    return first <= last ? { firstHour: mint(first as TrackHour), lastHour: mint(last as TrackHour) } : null;
  }, [gated, track]);
  const follow = useCallback((query: Readonly<Record<string, unknown>> | null) => gated !== null && follows && query ? query as FollowQuery : null, [gated, follows]);
  const time = useCallback((chosen?: GatedWindow, mode: 'pin' | 'range' = 'pin') => {
    const held = chosen ?? (gated === null ? null : { firstHour: gated, lastHour: gated });
    if (!track || !held || gated === null) return null;
    if (!chosen && follows) return mintTime({ mode: 'follow' });
    const first = track.branch.firstHour, offsets = { firstOffset: held.firstHour - first, lastOffset: held.lastHour - first };
    return mintTime(mode === 'range' ? { mode, ...offsets } : { mode, windows: { [track.id]: offsets } });
  }, [track, follows, gated]);
  const bind = useCallback((tag: string, extent?: GatedWindow) => {
    if (!track || gated === null) return null;
    const first = track.branch.firstHour;
    const chosen = choiceAt(track.source.metadata.campaignKey, track.branch.id, first, tag, gated);
    return boundCountryToken(bound(extent ? { ...chosen, extent: [extent.firstHour - first, extent.lastHour - first] } : chosen));
  }, [track, gated]);
  return { region, track, active, hour: gated, window, outside, stage, open, country, members, groups, hourAt, recorded, within, span, follow, time, bind };
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
  return Object.keys(windows).length ? mintTime({ mode: 'pin', windows }) : null;
}

export function useCampaignGate(runtime: Runtime, widget: WidgetSpec, active = true): Gate {
  const campaign = useMemo(() => ({ ...widget, time: { mode: 'campaign' as const } }), [widget]);
  return useRegionGate(runtime, useRegion(campaign), active);
}

export function useGate(runtime: Runtime, widget: Region['widget'], time: RegionAction = SPEC, active = true, needs?: GateNeeds): Gate {
  return useRegionGate(runtime, useRegion(widget, time), active, needs);
}
