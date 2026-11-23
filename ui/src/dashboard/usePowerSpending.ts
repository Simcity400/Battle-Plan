import { countryFromKey } from './countryKey.ts';
import { useMemo } from 'react';
import type { HourSpan, PowerSpendingReply } from '../generated/dashboard';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { defaultCountryKey } from './countrySelection.ts';
import { countryMatch, countryMatchFilter, resolveCountries } from './groups.ts';
import { useProducts } from './products.ts';
import { categoryChoices, ledgerOf, spendingChunks, spendingSpan, validatePowerSpending } from './powerSpending.ts';
export { SPENDING_CHUNK_HOURS, spendingChunks, structureAt, structureHours, structureParts } from './powerSpending.ts';
import type { PowerCurrency, SpendingSelection } from './powerSpending';
import { useCountryGroups } from './useCountryGroups.ts';
import { useGate } from './regionGate.ts';
import { SPEC } from './regionTime.ts';
import { productRequest } from './request.ts';
import { gatedRead, readData } from './gatedRead.ts';

const NO_WARNINGS: readonly string[] = [];

export function usePowerSpending(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, currency: PowerCurrency) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
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
  const tags = useMemo(() => !match.ready ? [] : selected.membership === null ? resolved.tags : [...new Set(selected.membership.map(run => run.tag))],
    [match.ready, selected, resolved.tags]);
  const gate = useGate(runtime, widget, SPEC, active);
  const span = track && gate.open ? spendingSpan(widget, track, runtime.tracks) : null;
  const asked = useMemo(() => track && span ? tags.flatMap(tag => spendingChunks(span.firstHour, span.lastHour).flatMap(([from, to]) => {
    const chunk = gate.span(from, to);
    return chunk ? [{ tag, request: productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'power-spending',
      params: { branchId: track.branch.id, country: countryFromKey(tag), currency, from: chunk.firstHour, to: chunk.lastHour } }) }] : [];
  })) : [],
  [track?.source.id, track?.source.revision, track?.branch.id, tags, currency, span?.firstHour, span?.lastHour, gate.span]);
  const requests = useMemo(() => asked.map(({ request }) => request), [asked]);
  const replies = useProducts<PowerSpendingReply>(runtime.client, requests, active, validatePowerSpending, track?.source);
  const ledgers = useMemo(() => {
    if (!replies.data) return undefined;
    const byTag = new Map<string, PowerSpendingReply[]>();
    replies.data.forEach((reply, at) => {
      const tag = asked[at].tag;
      const held = byTag.get(tag); if (held) held.push(reply); else byTag.set(tag, [reply]);
    });
    return tags.map(tag => ledgerOf(tag, currency, byTag.get(tag) ?? []));
  }, [replies.data, requests, tags, currency]);
  const read = gatedRead(gate, { kind: 'dated', data: ledgers, error: replies.error, retry: replies.retry, hour: span?.lastHour ?? null }, null, widget.time.mode !== 'pin');
  return { track, gate, read, tags, ledgers: readData(read), span, selection, ready: match.ready, reason: resolved.reason ?? match.reason,
    groupWarnings: groups.data?.warnings ?? NO_WARNINGS,
    error: replies.error ?? (groups.data ? undefined : groups.error),
    groupError: groups.data ? groups.error : undefined,
    retry: () => { replies.retry(); groups.retry(); } };
}

export function useSpendingChoices(widget: WidgetSpec, runtime: WidgetRuntime, currency: PowerCurrency): OptionChoices {
  const { ledgers } = usePowerSpending(widget, runtime, true, currency);
  return { categories: categoryChoices(ledgers) };
}
