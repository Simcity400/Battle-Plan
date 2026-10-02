/** Which look (name, map colour, flag) a chart shows for a country while looks are
 * read near the moment shown. Pure, for `useCountryLooks`. */
import { FAILED_FACT } from './countryFacts.ts';

/** What is known of a country's look: each part as last read, null when its read
 * gave nothing (the recording has none, or it cannot be had, such as flags without
 * the game folders). */
export interface Look { name: string | null; color: [number, number, number] | null; flag: string | null }

/** The reads of one country at the hour wanted, each part on its own: its value;
 * null once the read completed without it; undefined while pending; `FAILED_FACT`
 * when the read failed, so an older value is never shown as the current one. */
export interface LookRead {
  style: { name: string | null; color: [number, number, number] | null } | null | undefined | typeof FAILED_FACT;
  flag: string | null | undefined | typeof FAILED_FACT;
}

const NOTHING: Look = { name: null, color: null, flag: null };

/** The looks to show after this render's reads, from those shown before. Each part
 * stands alone: a completed read replaces its part (with nothing known when it gave
 * nothing, so an old value is never passed off as the current one), and a part
 * still being read keeps the one shown before, so nothing flashes while playback
 * moves on. A style read thus applies however the flag read goes. */
export function nextLooks(shown: ReadonlyMap<string, Look>, tags: readonly string[], read: (tag: string) => LookRead): Map<string, Look> {
  const next = new Map(shown);
  for (const tag of tags) {
    const { style, flag } = read(tag);
    if (style === undefined && flag === undefined) continue;
    const look = { ...(shown.get(tag) ?? NOTHING) };
    if (style !== undefined) { look.name = style === FAILED_FACT ? null : style?.name ?? null; look.color = style === FAILED_FACT ? null : style?.color ?? null; }
    if (flag !== undefined) look.flag = flag === FAILED_FACT ? null : flag;
    next.set(tag, look);
  }
  return next;
}

/** Keep a sampled look only in the same known country segment, including on seeks. */
export function retainLooks(shown: ReadonlyMap<string, Look>, before: ReadonlyMap<string, number>, at: ReadonlyMap<string, number>): Map<string, Look> {
  return new Map([...shown].filter(([tag]) => at.has(tag) && before.get(tag) === at.get(tag)));
}

/** A canvas may move its visible end before React has replanned its reads. Even
 * for that frame, a sampled/held look must belong to the identity being labelled. */
export function matchesLookSegment(segments: ReadonlyMap<string, number>, tag: string, start: number | null): boolean {
  return start !== null && segments.get(tag) === start;
}

/** The hook's render decision: drop other identities, then apply completed parts
 * at the sample hour while retaining parts whose reads are still pending. */
export function sampledLooks(before: { looks: ReadonlyMap<string, Look>; segments: ReadonlyMap<string, number> },
  samples: readonly { tag: string; hour: number; stretch: number }[], read: (sample: { tag: string; hour: number }) => LookRead,
  segments = new Map(samples.map(sample => [sample.tag, sample.stretch]))) {
  const reads = new Map(samples.map(sample => [sample.tag, read(sample)]));
  const looks = nextLooks(retainLooks(before.looks, before.segments, segments), [...reads.keys()], tag => reads.get(tag)!);
  return { looks, segments };
}
