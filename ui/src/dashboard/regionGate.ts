import { useEffect, useMemo, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import type { Failure } from './queryError';
import type { CountryIdentityHistory } from '../generated/dashboard';
import { defaultCountryKey, selectedCountryKeys } from './countrySelection.ts';
import { countryFromKey } from './countryKey.ts';
import { useSlotTags } from './countrySlots.ts';
import { choiceAt, resolveChoice, type CountryHolder, type ReachedCountry } from './reachedCountry.ts';
import { datedCountryToken, readDatedCountry, type DatedCountrySelection } from './datedCountry.ts';
import { worldFilterQuery, worldRequestTags } from './groups.ts';
import { useCountryGroups } from './useCountryGroups';
import { regionTrack, SPEC, type Outside, type Region, type RegionAction, type TrackHour } from './regionTime.ts';
import { useRegion, useRegionHour, useRegionOutside } from './widgetTime.ts';

export interface CountryNeed { stored: unknown; setOption?: (key: string, value: string) => void }
export interface GateNeeds { country?: CountryNeed; members?: boolean }

export interface CountryAt extends ReachedCountry {
  holders: CountryHolder[] | null; pick: (holder: CountryHolder) => void; error?: Failure; retry: () => void; historyHeld: boolean;
}
export interface MembersAt {
  countries: ReadonlySet<string> | null | undefined; requestTags: string[] | null | undefined; selectionReason: string | null; known: boolean;
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
  hour: TrackHour | null; outside: Outside | null; stage: GateStage; open: boolean;
  country: CountryAt | null; members: MembersAt | null; groups: ReturnType<typeof useCountryGroups>;
}

export function identityEpochs(histories: readonly CountryIdentityHistory[] | undefined): readonly number[] {
  const edges = new Set<number>();
  for (const history of histories ?? []) for (const run of history.identities) { edges.add(run.firstHour); edges.add(run.lastHour + 1); }
  return [...edges].sort((a, b) => a - b);
}

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
  const slots = useSlotTags(runtime.client, track, on && bySlot && hour !== null);
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
    need?.setOption?.('country', datedCountryToken(selection));
  };
  const fresh = need && !bySlot && !held && chosen && track && hour !== null && campaign !== undefined
    ? choiceAt(campaign, track.branch.id, track.branch.firstHour, chosen, hour) : null;
  const selection = bySlot ? null : held ?? fresh;
  const resolved = useMemo(() => selection && histories && track && hour !== null
    ? resolveChoice(histories, groups.complete, selection, track.branch.firstHour, track.branch.lastHour, hour) : null,
  [selection, histories, groups.complete, track, hour]);
  const kept = fresh ? datedCountryToken(fresh) : resolved?.rebind ? datedCountryToken(resolved.rebind) : null;
  useEffect(() => {
    const next = fresh ?? resolved?.rebind;
    if (next) keep(next);
  }, [kept]);
  if (!need) return null;
  const slotTag = bySlot && slots && hour !== null ? slots.tagAt(slot, hour) : null;
  const country: ReachedCountry = !chosen ? { chosen: null, tag: null, absent: false, reason: null }
    : bySlot ? { chosen, tag: slotTag, absent: !!slots && slotTag === null, reason: null }
    : resolved?.country ?? { chosen, tag: null, absent: false, reason: null };
  return { ...country, holders: resolved?.holders ?? null,
    pick: holder => { if (campaign !== undefined && branch !== undefined && chosen && track)
      keep({ campaign, branch, tag: chosen, originOffset: holder.origin - track.branch.firstHour }); },
    error: groups.error, retry: groups.retry, historyHeld: bySlot || !!histories };
}

export function useRegionGate(runtime: Runtime, region: Region, active: boolean, needs: GateNeeds = {}): Gate {
  const track = regionTrack(region, runtime.tracks);
  const widget = region.widget;
  const outside = useRegionOutside(runtime, region, active);
  const covered = !!track && outside === null;
  const wantsMembers = !!needs.members && widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, covered && (!!needs.country || wantsMembers) ? track : undefined, active);
  const fallback = defaultCountryKey(track);
  const lastHour = track?.branch.lastHour ?? null;
  const query = useMemo(() => wantsMembers && groups.data && lastHour !== null
    ? worldFilterQuery(widget.countries, fallback ? [fallback] : [], groups.data, lastHour) : null,
  [wantsMembers, widget.countries, fallback, groups.data, lastHour]);
  const request = useMemo(() => !needs.members || lastHour === null ? null
    : widget.countries.kind === 'all' ? worldRequestTags(widget.countries, [], undefined, lastHour)
    : groups.data ? worldRequestTags(widget.countries, fallback ? [fallback] : [], groups.data, lastHour) : null,
  [needs.members, widget.countries, fallback, groups.data, lastHour]);
  const histories = needs.country ? groups.data?.countryIdentities : undefined;
  const identity = useMemo(() => histories ? identityEpochs(histories) : NO_EDGES, [histories]);
  const epochs = useMemo(() => mergedEdges(identity, query?.edges ?? NO_EDGES), [identity, query]);
  const hour = useRegionHour(runtime, region, active, epochs);
  const country = useCountryAt(region, runtime, track, groups, covered ? hour : null, active, needs.country);
  const members = useMemo<MembersAt | null>(() => {
    if (!needs.members) return null;
    const filter = hour === null ? null : widget.countries.kind === 'all' ? { tags: null, reason: null, known: true } : query?.at(hour) ?? null;
    return { countries: filter && !filter.reason ? filter.tags : undefined,
      requestTags: request?.ready ? request.tags.length ? request.tags : null : undefined,
      selectionReason: filter ? filter.reason ?? request?.reason ?? null : null, known: filter?.known ?? false };
  }, [needs.members, hour, widget.countries.kind, query, request]);
  const stage = gateStage(track, outside, hour, country, needs.country !== undefined, wantsMembers, groups, members);
  return { region, track, active, hour: covered ? hour : null, outside, stage, open: stage.stage === 'open', country, members, groups };
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

export function gatedParams<P>(gate: Pick<Gate, 'open'>, params: P | null): P | null {
  return gate.open ? params : null;
}

export function useGate(runtime: Runtime, widget: Region['widget'], time: RegionAction = SPEC, active = true, needs?: GateNeeds): Gate {
  return useRegionGate(runtime, useRegion(widget, time), active, needs);
}
