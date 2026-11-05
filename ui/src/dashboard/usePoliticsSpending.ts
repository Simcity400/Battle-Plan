import { useMemo } from 'react';
import type { HourSpan, IdeaHistoryReply, PoliticsLedgerReply } from '../generated/dashboard';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { defaultCountryTag } from './countrySelection';
import { countryMatch, countryMatchFilter, resolveCountries } from './groups';
import { useFollowedProducts } from './products';
import { categoryChoices } from './politicsSpending.ts';
import { validatePowerBalance } from './powerBalance.ts';
import type { PowerCurrency, SpendingSelection } from './politicsSpending';
import { useCountryGroups } from './useCountryGroups';
import { useIdeaTooltips } from './useIdeaTooltips';
import { resolveWindow } from './model';
import { usePlaybackHour } from './playback.ts';

const recordedIdentity = (tag: string) => tag;
const NO_WARNINGS: readonly string[] = [];

/** Selected windows use prepared daily runs and dated payments. */
export function usePoliticsSpending(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, totals = false, choices = false) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const match = useMemo(() => countryMatch(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const selected = useMemo(() => countryMatchFilter(widget.countries, fallback ? [fallback] : [], groups.data,
    track?.branch.lastHour ?? 0, undefined, track?.branch.firstHour), [widget.countries, fallback, groups.data, track?.branch]);
  const selection = useMemo<SpendingSelection>(() => {
    const membership = selected.membership === null ? null : new Map<string, HourSpan[]>();
    if (membership) for (const run of selected.membership!) {
      const spans = membership.get(run.tag);
      if (spans) spans.push(run); else membership.set(run.tag, [run]);
    }
    return { matches: match.matches, membership, unobserved: selected.unobserved,
      requireRecordedHolder: widget.countries.kind === 'tags' || widget.countries.kind === 'selected' };
  }, [match.matches, selected, widget.countries.kind]);
  // Ledgers filter literal tags. Fetch every dated alias, keeping the original
  // choices in the native matcher and clipping coverage to their holder spans.
  const tags = useMemo(() => !match.ready ? [] : selected.membership === null ? resolved.tags
    : [...new Set(selected.membership.map(run => run.tag))], [match.ready, selected, resolved.tags]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const from = widget.time.mode === 'range' ? window?.firstHour : undefined;
  const hour = choices ? track?.branch.lastHour : window?.lastHour;
  const end = widget.time.mode === 'range' ? hour : track?.branch.lastHour;
  const hours = totals && !choices && end !== hour ? [hour, end] : [hour];
  const requests = track && hour != null ? hours.flatMap(hour => tags.map(tag => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'politics-ledger', params: { branchId: track.branch.id, tag, view: 'spending', from, hour, totals,
      within: selection.membership?.get(tag) } }))) : [];
  // The last hour's ledgers stay shown while playback's newest hour is read.
  const ledger = useFollowedProducts<PoliticsLedgerReply>(runtime.client, requests, ['hour'], active, validatePowerBalance);
  // Stable while the replies are: row motion commits a new history for every new array.
  const data = useMemo(() => ledger.data?.slice(0, tags.length), [ledger.data, tags.length]);
  const finalData = useMemo(() => hours.length > 1 ? ledger.data?.slice(tags.length, tags.length * 2) : data,
    [ledger.data, tags.length, hours.length, data]);
  // The hour the shown ledgers answer, which ledgers kept while the next hour loads hold.
  const shownHour = (ledger.requests?.[0]?.params as { hour?: number } | undefined)?.hour ?? hour;
  return { track, tags, ...ledger, data, finalData, shownHour,
    selection, ready: match.ready, reason: resolved.reason ?? match.reason, known: match.known,
    groupWarnings: groups.data?.warnings ?? NO_WARNINGS,
    error: ledger.error ?? (groups.data ? undefined : groups.error),
    groupError: groups.data ? groups.error : undefined,
    retry: () => { ledger.retry(); groups.retry(); } };
}

export function useSpendingChoices(widget: WidgetSpec, runtime: WidgetRuntime, currency: PowerCurrency): OptionChoices {
  const { data } = usePoliticsSpending(widget, runtime, true, true, true);
  return { categories: categoryChoices(data, currency) };
}

/** Tooltips reuse the ledger's shared idea effects. Country names in effects are requested at the action's own hour;
 * the fallback is its recorded tag, never a metadata name from another hour. */
export function useSpendingIdeaTooltips(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, replies: readonly PoliticsLedgerReply[] | undefined) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const merged = useMemo<Pick<IdeaHistoryReply, 'ideas' | 'artwork' | 'modifiers' | 'texts'> | undefined>(() => {
    const data = (replies ?? []).flatMap(reply => reply.ideaTooltips ? [reply.ideaTooltips] : []);
    if (!data.length) return undefined;
    const unique = <T,>(values: readonly T[], key: (value: T) => string) => [...new Map(values.map(value => [key(value), value])).values()];
    return { ideas: unique(data.flatMap(reply => reply.ideas), idea => idea.token),
      artwork: unique(data.flatMap(reply => reply.artwork), art => art.token), modifiers: unique(data.flatMap(reply => reply.modifiers), modifier => modifier.key),
      texts: unique(data.flatMap(reply => reply.texts), text => text.key) };
  }, [replies]);
  return useIdeaTooltips(merged, track, runtime.client, active, recordedIdentity);
}
