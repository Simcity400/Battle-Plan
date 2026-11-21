import { failureKey } from './queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from './client';
import { flagReadMetadata } from './countryFlagsAt.ts';
import type { CountryLabel } from './countryFlagsAt';
import { useCountryFlagsRead } from './countryFlags.ts';
import type { FlagRead } from './countryFlags';
import { nameKey, useCountryFactsByKey } from './countryNamesAt.ts';
import { FAILED_FACT, MAX_NAME_HOURS, isFailedFact } from './countryFacts.ts';
import { lookPlan } from './countryLooks.ts';
import type { NameAt } from './countryFacts';
import type { LookRead } from './looks';
import type { BoundTrack } from './model';
import { countryReadName, STYLES } from './useCountriesAt.ts';

export interface HoverRead {
  style: LookRead['style'];
  flag: FlagRead | null | undefined | typeof FAILED_FACT | Error;
}

export function useHoverLabels(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, active: boolean,
  pending: CountryLabel, missingName?: string, segmentAt?: (tag: string, hour: number) => number | null) {
  const [asked, setAsked] = useState<{ hour: number; tags: readonly string[] } | null>(null);
  const key = useRef('');
  const ask = useCallback((hour: number, tags: readonly string[]) => {
    const next = JSON.stringify([hour, tags]);
    if (next === key.current) return;
    key.current = next;
    queueMicrotask(() => setAsked({ hour, tags }));
  }, []);
  useEffect(() => { if (!active) { key.current = ''; setAsked(null); } }, [active]);
  const samples = useMemo(() => new Map(asked && segmentAt ? lookPlan(track?.branch, asked.tags, asked.hour, segmentAt).samples.map(sample => [sample.tag, sample.hour]) : []),
    [asked, track?.branch, segmentAt]);
  const wanted = useMemo<NameAt[]>(() => asked ? [...asked.tags.map(tag => ({ hour: asked.hour, tag })),
    ...[...samples].flatMap(([tag, hour]) => hour === asked.hour ? [] : [{ hour, tag }])] : [], [asked, samples]);
  const askedTags = useMemo(() => new Set(asked?.tags), [asked]);
  const maxHours = useMemo(() => Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size), [wanted]);
  const styles = useCountryFactsByKey(client, track, wanted, active, STYLES, maxHours);
  const flags = useCountryFlagsRead(client, track, wanted, active);
  const readOf = useCallback((tag: string): HoverRead => {
    const at = (hour: number | undefined): HoverRead => hour === undefined ? { style: undefined, flag: undefined }
      : { style: styles.get(nameKey(hour, tag)), flag: flags.get(nameKey(hour, tag)) };
    return asked ? heldRead(at(asked.hour), at(samples.get(tag))) : at(undefined);
  }, [asked, samples, styles, flags]);
  const labelAt = useCallback((hour: number): CountryLabel => tag => {
    if (!asked || asked.hour !== hour || !askedTags.has(tag)) return pending(tag);
    const { style, flag } = readOf(tag);
    const metadata = flagReadMetadata(flag);
    return {
      ...countryReadName(style, { name: 'Loading name\u2026' }, missingName ?? 'Country name unavailable'),
      ...metadata,
    };
  }, [asked, askedTags, readOf, pending, missingName]);
  const revision = useMemo(() => hoverRevision(asked, readOf), [asked, readOf]);
  return { ask, labelAt, revision };
}

export function heldRead(exact: HoverRead, sample: HoverRead): HoverRead {
  return {
    style: exact.style === undefined && sample.style && !isFailedFact(sample.style) ? sample.style : exact.style,
    flag: exact.flag === undefined && sample.flag && !isFailedFact(sample.flag) && sample.flag.url ? sample.flag : exact.flag,
  };
}

export function hoverRevision(asked: { hour: number; tags: readonly string[] } | null, read: (tag: string) => HoverRead): string {
  if (!asked) return '';
  const part = <V>(value: V | null | undefined | typeof FAILED_FACT | Error, show: (value: V) => unknown) => value === undefined ? 0 : value === null ? 1 : isFailedFact(value) ? failureKey(value) : show(value);
  return JSON.stringify([asked.hour, asked.tags, asked.tags.map(tag => {
    const { style, flag } = read(tag);
    return [part(style, value => [value.name, value.color]), part(flag, value => [value.url ? `${value.url.length}:${value.url.slice(-24)}` : null, value.reason])];
  })]);
}
