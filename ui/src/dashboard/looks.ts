/** Which look (name, map colour, flag) a chart shows for a country while looks are
 * read near the moment shown. Pure, for `useCountryLooks`. */

/** What is known of a country's look: each part as last read, null when its read
 * gave nothing (the recording has none, or it cannot be had, such as flags without
 * the game folders). */
export interface Look { name: string | null; color: [number, number, number] | null; flag: string | null }

/** The reads of one country at the hour wanted, each part on its own: its value;
 * null once the read completed without it; undefined while it is still being read
 * (or tried again after a failure). */
export interface LookRead {
  style: { name: string | null; color: [number, number, number] | null } | null | undefined;
  flag: string | null | undefined;
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
    if (style !== undefined) { look.name = style?.name ?? null; look.color = style?.color ?? null; }
    if (flag !== undefined) look.flag = flag;
    next.set(tag, look);
  }
  return next;
}
