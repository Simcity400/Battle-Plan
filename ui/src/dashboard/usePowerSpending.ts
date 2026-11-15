import { useMemo } from 'react';
import type { HourSpan, PowerSpendingReply } from '../generated/dashboard';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { defaultCountryTag } from './countrySelection.ts';
import { countryMatch, countryMatchFilter, resolveCountries } from './groups.ts';
import { useProducts } from './products.ts';
import { categoryChoices, ledgerOf, spendingChunks, spendingSpan, validatePowerSpending } from './powerSpending.ts';
export { SPENDING_CHUNK_HOURS, spendingChunks, structureAt, structureHours, structureParts } from './powerSpending.ts';
import type { PowerCurrency, SpendingSelection } from './powerSpending';
import { useCountryGroups } from './useCountryGroups.ts';

const NO_WARNINGS: readonly string[] = [];

/** One currency's ledgers of the widget's countries, each read once over the widget's span in aligned chunks (never by the
 * head's hour: a reader cuts them where it shows), with the group membership the rows are counted under. */
export function usePowerSpending(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, currency: PowerCurrency) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryTag(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, lastHour]);
  const match = useMemo(() => countryMatch(widget.countries, fallback ? [fallback] : [], groups.data, lastHour),
    [widget.countries, fallback, groups.data, lastHour]);
  const selected = useMemo(() => countryMatchFilter(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, undefined, track?.branch.firstHour),
    [widget.countries, fallback, groups.data, lastHour, track?.branch.firstHour]);
  const selection = useMemo<SpendingSelection>(() => {
    const membership = selected.membership === null ? null : new Map<string, HourSpan[]>();
    if (membership) for (const run of selected.membership!) {
      const spans = membership.get(run.tag);
      if (spans) spans.push(run); else membership.set(run.tag, [run]);
    }
    return { matches: match.matches, membership, unobserved: selected.unobserved };
  }, [match.matches, selected]);
  // Every dated alias of the choice is asked; the matcher keeps each to its own hours.
  const tags = useMemo(() => !match.ready ? [] : selected.membership === null ? resolved.tags : [...new Set(selected.membership.map(run => run.tag))],
    [match.ready, selected, resolved.tags]);
  const span = track ? spendingSpan(widget, track, runtime.tracks) : null;
  const requests = useMemo(() => track && span ? tags.flatMap(tag => spendingChunks(span.firstHour, span.lastHour).map(([from, to]) => ({
    sourceId: track.source.id, revision: track.source.revision, product: 'power-spending',
    params: { branchId: track.branch.id, tag, currency, from, to } }))) : [],
  // eslint-disable-next-line react-hooks/exhaustive-deps
  [track?.source.id, track?.source.revision, track?.branch.id, tags, currency, span?.firstHour, span?.lastHour]);
  const replies = useProducts<PowerSpendingReply>(runtime.client, requests, active, validatePowerSpending, track?.source);
  const ledgers = useMemo(() => {
    if (!replies.data) return undefined;
    const byTag = new Map<string, PowerSpendingReply[]>();
    for (const reply of replies.data) { const held = byTag.get(reply.tag); if (held) held.push(reply); else byTag.set(reply.tag, [reply]); }
    return tags.map(tag => ledgerOf(tag, currency, byTag.get(tag) ?? []));
  }, [replies.data, tags, currency]);
  return { track, tags, ledgers, span, selection, ready: match.ready, reason: resolved.reason ?? match.reason,
    groupWarnings: groups.data?.warnings ?? NO_WARNINGS,
    error: replies.error ?? (groups.data ? undefined : groups.error),
    groupError: groups.data ? groups.error : undefined,
    retry: () => { replies.retry(); groups.retry(); } };
}

export function useSpendingChoices(widget: WidgetSpec, runtime: WidgetRuntime, currency: PowerCurrency): OptionChoices {
  const { ledgers } = usePowerSpending(widget, runtime, true, currency);
  return { categories: categoryChoices(ledgers) };
}
