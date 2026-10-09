import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from './client';
import { flagReadMetadata } from './countryFlagsAt.ts';
import type { CountryLabel, FlagRead } from './countryFlagsAt';
import { nameKey, useCountryFactsRead } from './countryNamesAt.ts';
import { FAILED_FACT } from './countryFacts.ts';
import type { NameAt } from './countryFacts';
import type { LookRead } from './looks';
import type { BoundTrack } from './model';
import { countryReadName, FLAGS, STYLES } from './useCountriesAt.ts';

export interface HoverRead {
  style: LookRead['style'];
  flag: FlagRead | null | undefined | typeof FAILED_FACT;
}

/** Countries in a tooltip, named and flagged as the recording showed them at the hour
 * the tooltip is about. A chart's tooltip callback asks for its countries (`ask`),
 * which is cheap to call every frame: only a new hour or set of countries starts a
 * read, and only the countries of the tooltip shown are asked for, so requests stay
 * bounded however far the pointer travels. Each part (name, flag) on its own, as the
 * kit's looks rule reads it (`LookRead`): while its read is going a country shows
 * `pending`'s (never a blank); once it completes, what it gave is shown, and a part it
 * did not give, or whose read failed, is not known (the tag; no flag),
 * never another hour's. Flag status describes this read even while the fallback URL
 * is held. `revision` changes as the question or any answer changes, for
 * the chart to redraw the tooltip. */
export function useHoverLabels(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, active: boolean,
  pending: CountryLabel, missingName?: string) {
  const [asked, setAsked] = useState<{ hour: number; tags: readonly string[] } | null>(null);
  const key = useRef('');
  // Called from a tooltip callback, outside React's render: a state change only when the question changes.
  const ask = useCallback((hour: number, tags: readonly string[]) => {
    const next = JSON.stringify([hour, tags]);
    if (next === key.current) return;
    key.current = next;
    queueMicrotask(() => setAsked({ hour, tags }));
  }, []);
  useEffect(() => { if (!active) { key.current = ''; setAsked(null); } }, [active]);
  const wanted = useMemo<NameAt[]>(() => asked ? asked.tags.map(tag => ({ hour: asked.hour, tag })) : [], [asked]);
  const askedTags = useMemo(() => new Set(asked?.tags), [asked]);
  const styles = useCountryFactsRead(client, track, wanted, active, STYLES);
  const flags = useCountryFactsRead(client, track, wanted, active, FLAGS);
  const readOf = useCallback((tag: string): HoverRead => asked ? { style: styles.get(nameKey(asked.hour, tag)), flag: flags.get(nameKey(asked.hour, tag)) }
    : { style: undefined, flag: undefined }, [asked, styles, flags]);
  const labelAt = useCallback((hour: number): CountryLabel => tag => {
    const fallback = pending(tag);
    if (!asked || asked.hour !== hour || !askedTags.has(tag)) return fallback;
    const { style, flag } = readOf(tag);
    const metadata = flagReadMetadata(flag);
    return {
      ...countryReadName(style, fallback, missingName ?? tag),
      ...metadata,
      flag: flag === undefined ? fallback.flag : metadata.flag,
    };
  }, [asked, askedTags, readOf, pending, missingName]);
  // Changes whenever the question or any answer for it changes, so a chart redraws its
  // tooltip even when one country's answers replace another's with maps of the same size.
  const revision = useMemo(() => hoverRevision(asked, readOf), [asked, readOf]);
  return { ask, labelAt, revision };
}

/** What a tooltip's labels depend on, as a string that changes when the question (the
 * hour and countries asked) or any answer (a name, a flag, a read finishing with none)
 * changes. A flag is fingerprinted by its length and end, not copied whole. */
export function hoverRevision(asked: { hour: number; tags: readonly string[] } | null, read: (tag: string) => HoverRead): string {
  if (!asked) return '';
  const part = <V>(value: V | null | undefined | typeof FAILED_FACT, show: (value: V) => unknown) => value === undefined ? 0 : value === null ? 1 : value === FAILED_FACT ? 2 : show(value);
  return JSON.stringify([asked.hour, asked.tags, asked.tags.map(tag => {
    const { style, flag } = read(tag);
    return [part(style, value => [value.name, value.color]), part(flag, value => [value.url ? `${value.url.length}:${value.url.slice(-24)}` : null, value.reason])];
  })]);
}
