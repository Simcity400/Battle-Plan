import { useMemo, useRef } from 'react';
import type { MovingRow } from '../../charts/rowMotion';
import type { DashboardClient } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import { lookHour } from '../../dashboard/countryLooks.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { BoundTrack } from '../../dashboard/model';
import { FOLLOW_MS } from '../../dashboard/useCountriesAt.ts';
import { useThrottled } from '../../dashboard/useThrottled.ts';
import type { RaceRow } from './decryptionData';

export interface CipherCountries { owner: ReturnType<CountryLabel>; target: ReturnType<CountryLabel>; hour?: number }
type Shown = CipherCountries & { row: RaceRow; hour: number };
const sameIdentity = (a: RaceRow, b: RaceRow) => a.key === b.key && a.pair.ownerId === b.pair.ownerId && a.pair.targetId === b.pair.targetId && a.tags.firstHour === b.tags.firstHour
  && a.tags.ownerTag === b.tags.ownerTag && a.tags.targetTag === b.tags.targetTag;
const missing = (id: string, tag: string | null): ReturnType<CountryLabel> => ({ name: tag ?? `Country ${id} (tag unavailable)` });

/** Exact sampled country reads, with one completed display pair held only inside
 * its native tag segment. The shared fact store owns requests and their cache. */
export function useDecryptionCountries(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined,
  rows: readonly RaceRow[], visible: readonly MovingRow<RaceRow>[], active: boolean) {
  const content = useContentRevision(client);
  const sampled = useThrottled(visible, FOLLOW_MS);
  const current = useMemo(() => new Map(rows.map(row => [row.key, row])), [rows]);
  const drawn = useMemo(() => new Set(visible.filter(row => row.phase !== 'leaving').map(row => row.key)), [visible]);
  const key = track && active ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : '';
  const shown = useRef<{ key: string; client: DashboardClient; pairs: Map<string, Shown>; reading: Map<string, RaceRow> }>({ key, client, pairs: new Map(), reading: new Map() });
  if (shown.current.key !== key || shown.current.client !== client) shown.current = { key, client, pairs: new Map(), reading: new Map() };
  const valid = (sample: RaceRow) => {
    const row = current.get(sample.key);
    if (!active || !track || !drawn.has(sample.key) || !row || sample.hour > row.hour || !sameIdentity(sample, row)) return false;
    const before = lookHour(track.branch, sample.hour), at = lookHour(track.branch, row.hour);
    return before !== null && at !== null && before.stretch === at.stretch;
  };
  const { pairs: held, reading } = shown.current;
  for (const [id, pair] of held) if (!valid(pair.row)) held.delete(id);
  for (const [id, row] of reading) if (!valid(row)) reading.delete(id);
  // A slow read stays wanted even across throttle ticks; a later cohort starts
  // only after this pair finishes, or its native identity becomes invalid.
  for (const { item, phase } of sampled) if (phase !== 'leaving' && valid(item) && !reading.has(item.key)) reading.set(item.key, item);
  const samples = [...reading.values()];
  const unique = new Map<string, { hour: number; tag: string }>();
  for (const row of samples) for (const tag of [row.tags.ownerTag, row.tags.targetTag]) {
    if (tag !== null) unique.set(JSON.stringify([row.hour, tag]), { hour: row.hour, tag });
  }
  const wanted = [...unique.values()];
  const maxHours = Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size);
  const countries = useCountryLabelsAt(client, track, wanted, active, maxHours);
  const pending = new Map<string, Shown>();
  for (const row of samples) {
    const label = countries.labelAt(row.hour);
    const owner = row.tags.ownerTag === null ? missing(row.pair.ownerId, null) : label(row.tags.ownerTag);
    const target = row.tags.targetTag === null ? missing(row.pair.targetId, null) : label(row.tags.targetTag);
    const ready = (tag: string | null, value: ReturnType<CountryLabel>) => tag === null || (value.known !== undefined && !value.flagPending);
    const pair = { owner, target, hour: row.hour, row };
    if (ready(row.tags.ownerTag, owner) && ready(row.tags.targetTag, target)) {
      held.set(row.key, pair);
      reading.delete(row.key);
    } else pending.set(row.key, pair);
  }
  return {
    pair: (row: RaceRow): CipherCountries => {
      const pair = held.get(row.key) ?? pending.get(row.key);
      if (pair && pair.hour <= row.hour && sameIdentity(pair.row, row)) return pair;
      const waiting = valid(row);
      const fallback = (id: string, tag: string | null) => tag === null ? missing(id, tag) : { ...missing(id, tag), flagPending: waiting };
      return { owner: fallback(row.pair.ownerId, row.tags.ownerTag), target: fallback(row.pair.targetId, row.tags.targetTag) };
    },
    revision: countries.revision,
  };
}
