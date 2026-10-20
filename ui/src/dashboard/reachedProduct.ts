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
import { reachedKeys, reachedView, settle, type ReachedState, type Settled } from './reachedState.ts';

export type { ReachedState };

/** A reached-hour product (one country or unit at the playback hour). Requests
 * are transient, so playback never pushes branch histories out of the shared
 * cache; a newer request cancels the one before. `subject` names what the
 * answer is about apart from the hour: a changed subject drops the old answer. */
export function useReachedProduct<T>(client: DashboardClient, source: DashboardSource | undefined, request: ProductRequest | null,
  subject: string, active: boolean, validate: (data: unknown) => T): ReachedState<T> {
  const content = useContentRevision(client);
  const { owner, key } = reachedKeys(request, subject, content);
  const [state, setState] = useState<{ client: DashboardClient; settled: Settled<T> }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !request || !key) return;
    const controller = new AbortController();
    const finish = (outcome: { data: T } | { error: string }) => {
      if (!controller.signal.aborted) setState(previous => ({ client,
        settled: settle(previous?.client === client ? previous.settled : undefined, owner, key, outcome) }));
    };
    loadTransientProduct<unknown>(client, request, controller.signal, source).then(validate).then(
      data => finish({ data }), error => finish({ error: error instanceof Error ? error.message : String(error) }));
    return () => controller.abort();
    // The structured key names the request; the request object is rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, key, owner, active, attempt]);
  return { ...reachedView(state?.client === client ? state.settled : undefined, owner, key),
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}

/** A single-country reached-hour widget's country at an hour. The choice is bound to the
 * hour it was made at, kept in the widget's `country` option ([`REACHED_COUNTRY_OPTION`])
 * as a dated choice, and followed through only the dated aliases of the identity holding
 * its tag then ([`resolveChoice`]). A new choice binds at the hour shown. Its rebinding,
 * or the `holders` to `pick` from, waits for the complete identity history. `error`
 * and `retry` are the identity history's. */
export function useReachedCountry(widget: WidgetSpec, runtime: WidgetRuntime, hour: number, active: boolean, stored: unknown,
  setOption?: (key: string, value: string) => void)
  : ReachedCountry & { holders: CountryHolder[] | null; pick: (holder: CountryHolder) => void; error?: string; retry: () => void } {
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
    error: groups.error, retry: groups.retry };
}

/** One country's name at an hour for a widget's own messages; its tag while the name loads. */
export function useCountryNameAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tag: string | null, hour: number, active: boolean): string {
  const wanted = useMemo(() => tag ? [{ hour, tag }] : [], [tag, hour]);
  const names = useCountryNamesAt(client, track, wanted, active);
  return tag ? names.get(nameKey(hour, tag)) ?? tag : '';
}
