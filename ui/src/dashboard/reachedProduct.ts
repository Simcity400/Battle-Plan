import { nameReadMetadata } from './nameReads.ts';
import { type Failure } from './queryError.ts';
import { useEffect, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';
import { useContentRevision } from './contentRevision';
import { loadTransientProduct } from './products';
import { nameKey, useCountryNamesAt } from './countryNamesAt';
import type { BoundTrack, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';
import { selectedCountryKeys } from './countrySelection.ts';
import { countryFromKey } from './countryKey.ts';
import { useSlotTags } from './countrySlots.ts';
import { choiceAt, outsideCountry, resolveChoice, type CountryHolder, type ReachedCountry } from './reachedCountry.ts';
import { datedCountryToken, readDatedCountry, type DatedCountrySelection } from './datedCountry.ts';
import { useCountryGroups } from './useCountryGroups';
import { reachedKeys, reachedView, type ReachedState } from './reachedState.ts';
import { useViewRead } from './viewRead';
import type { CountryIdentityHistory } from '../generated/dashboard';
import type { RegionAction, TrackHour } from './regionTime.ts';
import { useRegion, useRegionHour } from './widgetTime.ts';

export type { ReachedState };

export function useReachedProduct<T>(client: DashboardClient, source: DashboardSource | undefined, request: ProductRequest | null,
  subject: string, active: boolean, validate: (data: unknown) => T): ReachedState<T> {
  const content = useContentRevision(client);
  const { owner, key } = reachedKeys(request, subject, content);
  const read = useViewRead<ProductRequest, T>(key === null ? null : owner, request, key ?? '', active,
    (asked, signal) => loadTransientProduct<unknown>(client, asked, signal, source).then(validate), client);
  return { ...reachedView(read, key), retry: read.retry };
}

export function identityEpochs(histories: readonly CountryIdentityHistory[] | undefined): readonly number[] | undefined {
  if (!histories) return undefined;
  const edges = new Set<number>();
  for (const history of histories) for (const run of history.identities) { edges.add(run.firstHour); edges.add(run.lastHour + 1); }
  return [...edges].sort((a, b) => a - b);
}

export function useReachedCountry(widget: WidgetSpec, runtime: WidgetRuntime, time: RegionAction, active: boolean, stored: unknown,
  setOption?: (key: string, value: string) => void)
  : ReachedCountry & { hour: TrackHour | null; holders: CountryHolder[] | null; pick: (holder: CountryHolder) => void; error?: Failure; retry: () => void; historyHeld: boolean } {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, active);
  const histories = groups.data?.countryIdentities;
  const epochs = useMemo(() => identityEpochs(histories), [histories]);
  const hour = useRegionHour(runtime, useRegion(widget, time), active, epochs);
  const chosen = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const slot = chosen === null ? null : countryFromKey(chosen);
  const bySlot = typeof slot === 'number';
  const slots = useSlotTags(runtime.client, track, active && bySlot);
  const campaign = track?.source.metadata.campaignKey, branch = track?.branch.id;
  const owner = JSON.stringify([campaign, branch, chosen]);
  const saved = useMemo(() => {
    const selection = campaign !== undefined && branch !== undefined ? readDatedCountry(stored, campaign, branch) : null;
    return selection?.tag === chosen ? selection : null;
  }, [stored, campaign, branch, chosen]);
  const [local, setLocal] = useState<{ owner: string; selection: DatedCountrySelection } | null>(null);
  const held = saved ?? (local?.owner === owner ? local.selection : null);
  const keep = (selection: DatedCountrySelection) => {
    setLocal({ owner, selection });
    setOption?.('country', datedCountryToken(selection));
  };
  const fresh = !bySlot && !held && chosen && track && hour !== null && campaign !== undefined ? choiceAt(campaign, track.branch.id, track.branch.firstHour, chosen, hour) : null;
  const selection = bySlot ? null : held ?? fresh;
  const resolved = useMemo(() => selection && histories && track && hour !== null
    ? resolveChoice(histories, groups.complete, selection, track.branch.firstHour, track.branch.lastHour, hour) : null,
  [selection, histories, groups.complete, track, hour]);
  const kept = fresh ? datedCountryToken(fresh) : resolved?.rebind ? datedCountryToken(resolved.rebind) : null;
  useEffect(() => {
    const next = fresh ?? resolved?.rebind;
    if (next) keep(next);
  }, [kept]);
  const slotTag = typeof slot === 'number' && slots && hour !== null ? slots.tagAt(slot, hour) : null;
  const country: ReachedCountry = !chosen ? { chosen: null, tag: null, absent: false, reason: null }
    : track && hour === null ? outsideCountry(chosen)
    : bySlot ? { chosen, tag: slotTag, absent: !!slots && slotTag === null, reason: null }
    : resolved?.country ?? { chosen, tag: null, absent: false, reason: null };
  return { ...country, hour, holders: resolved?.holders ?? null,
    pick: holder => { if (campaign !== undefined && branch !== undefined && chosen && track)
      keep({ campaign, branch, tag: chosen, originOffset: holder.origin - track.branch.firstHour }); },
    error: groups.error, retry: groups.retry, historyHeld: bySlot || !!histories };
}

export function useCountryNameAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tag: string | null, hour: number | null, active: boolean) {
  const available = !!track && !!tag && hour !== null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const wanted = useMemo(() => available && tag && hour !== null ? [{ hour, tag }] : [], [available, tag, hour]);
  const names = useCountryNamesAt(client, track, wanted, active && available);
  return nameReadMetadata(available && tag && hour !== null ? names.get(nameKey(hour, tag)) : undefined);
}
