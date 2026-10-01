import { useMemo, useRef } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { flagSpec } from './countryFlagsAt.ts';
import { nameKey, useCountryFactsRead } from './countryNamesAt.ts';
import type { FactSpec } from './countryFacts';
import { nextLooks } from './looks.ts';
import type { Look } from './looks';
import type { BoundTrack } from './model';

/** Country names and colours are read at most once per this many hours of
 * playback, so following playback asks for them about once a month. */
export const LOOK_STEP = 30 * 24;

/** A branch's recorded hours: from its first to its last, less its gaps. */
export interface RecordedBranch { firstHour: number; lastHour: number; gaps: readonly { firstHour: number; lastHour: number }[] }

/** Where looks are read for a moment shown: a recorded hour of the same unbroken
 * stretch of the recording as the moment (`stretch`, the stretch's first hour), the
 * branch's first hour plus whole steps where that is in the stretch, else the
 * stretch's first hour; never an hour of a gap, so a read never waits on hours the
 * recording lacks. Null when the moment itself is not recorded (in a gap, or
 * outside the branch). */
export function lookHour(branch: RecordedBranch, hour: number): { hour: number; stretch: number } | null {
  const moment = Math.floor(hour);
  if (moment < branch.firstHour || moment > branch.lastHour) return null;
  if (branch.gaps.some(gap => gap.firstHour <= moment && moment <= gap.lastHour)) return null;
  // The stretch starts after the last gap before the moment.
  const stretch = branch.gaps.reduce((start, gap) => gap.lastHour < moment ? Math.max(start, gap.lastHour + 1) : start, branch.firstHour);
  const step = branch.firstHour + Math.floor((moment - branch.firstHour) / LOOK_STEP) * LOOK_STEP;
  return { hour: Math.max(step, stretch), stretch };
}

/** Styles from `country-styles`, which names at most 64 countries per request. */
const STYLES: FactSpec<CountryStyle> = {
  product: 'country-styles', perRequest: 64,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.map(style => [style.tag, style] as const)),
};
const FLAGS = flagSpec('small');

export interface CountryLooks {
  /** The name as recorded near the moment, else the catalogue's name, else the tag. */
  name: (tag: string) => string;
  /** The recorded map colour as sRGB bytes, or null when not known. */
  color: (tag: string) => [number, number, number] | null;
  /** The flag as an image URL, or undefined when not known. */
  flag: (tag: string) => string | undefined;
  /** Changes as looks arrive, for a chart to redraw. */
  revision: number;
}

/** Names, map colours and flags of `tags` as recorded near `hour` (`lookHour`), for
 * charts that follow playback with many countries (a race, lines). Each part is read
 * on its own: while its next read is loading a country keeps the part shown before,
 * so nothing flashes; once a read completes without it, it is not known and the
 * chart shows its fallback, never an older one. Looks are kept within one recorded
 * stretch only: another track, or a moment past a gap, starts afresh. */
export function useCountryLooks(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tags: readonly string[], hour: number,
  active: boolean): CountryLooks {
  const at = track ? lookHour(track.branch, hour) : null;
  // The set of countries, not their order: a race re-ranking asks for nothing new.
  const set = at ? [...new Set(tags)].sort() : [];
  const tagKey = JSON.stringify(set);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const wanted = useMemo(() => at ? set.map(tag => ({ hour: at.hour, tag })) : [], [tagKey, at?.hour]);
  const styles = useCountryFactsRead(client, track, wanted, active, STYLES);
  const flags = useCountryFactsRead(client, track, wanted, active, FLAGS);
  const heldKey = track && at ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, at.stretch]) : '';
  const shown = useRef<{ key: string; looks: Map<string, Look> }>({ key: heldKey, looks: new Map() });
  if (shown.current.key !== heldKey) shown.current = { key: heldKey, looks: new Map() };
  const held = shown.current;
  return useMemo(() => {
    // Merging what finished is idempotent, so a repeated render changes nothing.
    const key = (tag: string) => nameKey(wanted[0]?.hour ?? 0, tag);
    held.looks = nextLooks(held.looks, wanted.map(item => item.tag), tag => ({ style: styles.get(key(tag)), flag: flags.get(key(tag)) }));
    const looks = held.looks;
    return {
      name: tag => looks.get(tag)?.name ?? track?.source.metadata.countries.find(country => country.tag === tag)?.name ?? tag,
      color: tag => looks.get(tag)?.color ?? null,
      flag: tag => looks.get(tag)?.flag ?? undefined,
      revision: styles.size + flags.size * 1000 + looks.size * 1_000_000,
    };
  }, [wanted, styles, flags, held, track]); // eslint-disable-line react-hooks/exhaustive-deps
}
