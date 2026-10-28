import { useEffect, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';
import { useContentRevision } from './contentRevision';
import { loadTransientProduct } from './products';
import { nameKey, useCountryNamesAt } from './countryNamesAt';
import type { BoundTrack, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';
import { selectedTags } from './countrySelection.ts';
import { choiceAt, resolveChoice, type CountryHolder, type ReachedCountry } from './reachedCountry.ts';
import { datedCountryToken, readDatedCountry, type DatedCountrySelection } from './datedCountry.ts';
import { useCountryGroups } from './useCountryGroups';
import { reachedKeys, reachedView, type ReachedState } from './reachedState.ts';
import { useViewRead } from './viewRead';

export type { ReachedState };

/** A reached-hour product (one country or unit at the playback hour). Requests
 * are transient, so playback never pushes branch histories out of the shared
 * cache. One read runs at a time per subject: a newer hour never cancels it, so
 * it lands and stays shown while the newest hour is read. `subject` names what
 * the answer is about apart from the hour: a changed subject cancels and drops
 * the old answer. */
export function useReachedProduct<T>(client: DashboardClient, source: DashboardSource | undefined, request: ProductRequest | null,
  subject: string, active: boolean, validate: (data: unknown) => T): ReachedState<T> {
  const content = useContentRevision(client);
  const { owner, key } = reachedKeys(request, subject, content);
  const read = useViewRead<ProductRequest, T>(key === null ? null : owner, request, key ?? '', active,
    (asked, signal) => loadTransientProduct<unknown>(client, asked, signal, source).then(validate), client);
  return { ...reachedView(read, key), retry: read.retry };
}

/** Rebinding waits for complete identity history; held history remains usable alongside errors. */
export function useReachedCountry(widget: WidgetSpec, runtime: WidgetRuntime, hour: number, active: boolean, stored: unknown,
  setOption?: (key: string, value: string) => void)
  : ReachedCountry & { holders: CountryHolder[] | null; pick: (holder: CountryHolder) => void; error?: string; retry: () => void; historyHeld: boolean } {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, active);
  const histories = groups.data?.countryIdentities;
  const chosen = selectedTags(widget.countries, track)[0] ?? null;
  const campaign = track?.source.metadata.campaignKey, branch = track?.branch.id;
  const owner = JSON.stringify([campaign, branch, chosen]);
  const saved = useMemo(() => {
    const selection = campaign !== undefined && branch !== undefined ? readDatedCountry(stored, campaign, branch) : null;
    return selection?.tag === chosen ? selection : null;
  }, [stored, campaign, branch, chosen]);
  // A dashboard that cannot keep options binds for this session.
  const [local, setLocal] = useState<{ owner: string; selection: DatedCountrySelection } | null>(null);
  const held = saved ?? (local?.owner === owner ? local.selection : null);
  const keep = (selection: DatedCountrySelection) => {
    setLocal({ owner, selection });
    setOption?.('country', datedCountryToken(selection));
  };
  const fresh = !held && chosen && track && campaign !== undefined ? choiceAt(campaign, track.branch.id, track.branch.firstHour, chosen, hour) : null;
  const selection = held ?? fresh;
  const resolved = useMemo(() => selection && histories && track
    ? resolveChoice(histories, groups.complete, selection, track.branch.firstHour, track.branch.lastHour, hour) : null,
  [selection, histories, groups.complete, track, hour]);
  // Keep a new choice's hour, and a rebinding the complete history settled, once each.
  const kept = fresh ? datedCountryToken(fresh) : resolved?.rebind ? datedCountryToken(resolved.rebind) : null;
  useEffect(() => {
    const next = fresh ?? resolved?.rebind;
    if (next) keep(next);
    // `kept` names the binding to keep; `keep` and the objects are rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kept]);
  const country: ReachedCountry = !chosen ? { chosen: null, tag: null, absent: false, reason: null }
    : resolved?.country ?? { chosen, tag: null, absent: false, reason: null };
  return { ...country, holders: resolved?.holders ?? null,
    pick: holder => { if (campaign !== undefined && branch !== undefined && chosen && track)
      keep({ campaign, branch, tag: chosen, originOffset: holder.origin - track.branch.firstHour }); },
    error: groups.error, retry: groups.retry, historyHeld: !!histories };
}

/** One country's name at an hour for a widget's own messages; its tag while the name loads. */
export function useCountryNameAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tag: string | null, hour: number, active: boolean): string {
  const wanted = useMemo(() => tag ? [{ hour, tag }] : [], [tag, hour]);
  const names = useCountryNamesAt(client, track, wanted, active);
  return tag ? names.get(nameKey(hour, tag)) ?? tag : '';
}
