import { useMemo } from 'react';
import type { CountryStyle } from '../generated/dashboard';
import type { ChartTokens } from '../charts/tokens';
import type { DashboardClient } from './client';
import { countryLineColours } from './countryLineColours.ts';
import { FAILED_FACT, MAX_NAME_HOURS, nameKey } from './countryFacts.ts';
import type { NameAt } from './countryFacts';
import { useCountryFactsRead } from './countryNamesAt.ts';
import { STYLES } from './useCountriesAt.ts';
import type { BoundTrack } from './model';

/** A dated read as `useCountryFactsRead` returns it: absent while it is still going. */
type Read<V> = V | null | typeof FAILED_FACT | undefined;

/** A dated name as a label shows it: the recorded name; loading only while its read is
 * still going; unavailable once the read completed without a name, or failed. */
export const nameReadText = (name: string | null | symbol | undefined) => typeof name === 'string' ? name : name === undefined ? 'Loading name…' : 'Country name unavailable';

/** One native country holder a widget draws: a key of its own, the tag it held, and an
 * hour it is known to have held that tag. Two holders of one tag are two keys. */
export interface HolderAt { key: string; tag: string; hour: number }

/** The reads `holders` need: each tag at its own holder's hour, each once. Every hour
 * wanted stays held, however many holders there are. */
export function holderReads(holders: readonly HolderAt[]): { wanted: NameAt[]; maxHours: number } {
  const wanted = new Map<string, NameAt>();
  for (const holder of holders) {
    const hour = Math.floor(holder.hour);
    wanted.set(nameKey(hour, holder.tag), { hour, tag: holder.tag });
  }
  const list = [...wanted.values()];
  return { wanted: list, maxHours: Math.max(MAX_NAME_HOURS, new Set(list.map(item => item.hour)).size) };
}

export interface HolderLooks {
  /** The holder's recorded name, else the loading or unavailable wording. */
  name: (key: string) => string;
  /** The holder's recorded map colour, else its palette colour, as the country charts colour. */
  colour: (key: string) => string;
  /** The holder's recorded map colour as sRGB bytes, or null when not known (yet). */
  color: (key: string) => CountryStyle['color'];
  /** Changes as reads arrive, for a chart to redraw. */
  revision: string;
}

/** Names and colours of holders from their dated style reads. Each holder is read
 * exactly at its own hour, never sampled or held from another hour, so a tag a later
 * country takes never lends its name or colour to an earlier holder. */
export function holderLooks(holders: readonly HolderAt[], styles: ReadonlyMap<string, Read<CountryStyle>>,
  tokens: Pick<ChartTokens, 'surface' | 'muted' | 'countryColours'>): HolderLooks {
  const reads = new Map(holders.map(holder => [holder.key, styles.get(nameKey(Math.floor(holder.hour), holder.tag))]));
  const found = new Map<string, CountryStyle>();
  for (const [key, style] of reads) if (typeof style === 'object' && style) found.set(key, style);
  const keys = [...reads.keys()].sort();
  const colours = countryLineColours(keys, found, tokens.surface, tokens.countryColours === 'fixed');
  return {
    name: key => {
      if (!reads.has(key)) return nameReadText(null);
      const style = reads.get(key);
      return nameReadText(typeof style === 'object' && style ? style.name : style === null ? null : style);
    },
    colour: key => colours.get(key) ?? tokens.muted,
    color: key => found.get(key)?.color ?? null,
    revision: keys.map(key => `${reads.get(key) === undefined ? '?' : found.get(key)?.color?.join(',') ?? '-'}:${found.get(key)?.name ?? ''}`).join('|'),
  };
}

/** `holderLooks` for the holders a widget draws, read from the recording. */
export function useHolderLooks(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, holders: readonly HolderAt[], active: boolean,
  tokens: Pick<ChartTokens, 'surface' | 'muted' | 'countryColours'>): HolderLooks {
  const plan = useMemo(() => holderReads(holders), [holders]);
  const styles = useCountryFactsRead(client, track, plan.wanted, active, STYLES, plan.maxHours);
  return useMemo(() => holderLooks(holders, styles, tokens), [holders, styles, tokens.surface, tokens.muted, tokens.countryColours]); // eslint-disable-line react-hooks/exhaustive-deps
}
