import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import type { Artwork, CategoryInfo, IdeaHistoryReply, IdeaInfo, IdeaSpan } from '../../generated/dashboard';

/** One idea-category timeline's lanes, from the idea-history reply: one lane per
 * idea slot, and one per seat of a character slot (three political advisor seats
 * are three lanes). Categories and slots are the content's own. */

export type LawStyle = 'both' | 'shade' | 'icons';

/** A recorded holder whose character details were not recorded. */
export const UNIDENTIFIED_HOLDER = 'Holder not recorded';
export type HolderStyle = 'both' | 'name' | 'portrait';

/** Categories a timeline can show: on the political screen, not hidden, with slots. */
export function timelineCategories(categories: readonly CategoryInfo[]): CategoryInfo[] {
  return categories.filter(category => category.politicsTab && !category.hidden && category.slots.length > 0);
}

/** The category a widget shows: its chosen one, or the first a timeline can show. */
export function chosenCategory(categories: readonly CategoryInfo[], chosen: string): CategoryInfo | null {
  const eligible = timelineCategories(categories);
  return chosen ? eligible.find(category => category.token === chosen) ?? null : eligible[0] ?? null;
}

export function lanesFor(reply: IdeaHistoryReply, category: CategoryInfo, styles: { laws: LawStyle; holders: HolderStyle }): TimelineLane[] {
  const infos = new Map<string, IdeaInfo>(reply.ideas.map(info => [info.token, info]));
  const art = new Map<string, Artwork>(reply.artwork.map(item => [item.token, item]));
  // An advisor span shows its own recorded holder, not the idea's latest one. A
  // holder the recording could not identify stays unnamed rather than borrowing
  // another holder's name and portrait.
  const holder = (token: string, character: number) => `${token}#${character}`;
  const holders = new Map<string, Artwork>(reply.holders.map(item => [holder(item.token, item.character), item.artwork]));
  const span = (item: IdeaSpan, shade?: number): TimelineSpan => {
    const base = { id: `${item.token}:${item.firstHour}`, firstHour: item.firstHour, lastHour: item.lastHour, shade };
    if (item.character) {
      const look = holders.get(holder(item.token, item.character.id));
      return look ? { ...base, label: look.name ?? item.token, icon: look.icon ?? undefined } : { ...base, label: UNIDENTIFIED_HOLDER };
    }
    const look = art.get(item.token);
    return { ...base, label: look?.name ?? item.token, icon: look?.icon ?? undefined };
  };
  const lanes: TimelineLane[] = [];
  for (const slot of category.slots) {
    const label = slot.name ?? slot.token;
    // The seat the recording gave a span comes first; content's slot otherwise.
    const held = reply.spans.filter(item => (item.slot ?? infos.get(item.token)?.slot) === slot.token).sort((a, b) => a.firstHour - b.firstHour);
    if (!slot.character) {
      const law = slot.laws.length > 0;
      const shade = (token: string) => {
        const level = slot.laws.indexOf(token);
        return law && styles.laws !== 'icons' && level >= 0 ? (slot.laws.length > 1 ? level / (slot.laws.length - 1) : 0) : undefined;
      };
      lanes.push({ id: slot.token, label, spans: held.map(item => span(item, shade(item.token))),
        icons: law ? styles.laws !== 'shade' : styles.holders !== 'name', names: law || styles.holders !== 'portrait' });
      continue;
    }
    // Seats fill in order: each holder takes the first seat free when it starts.
    const seats: TimelineSpan[][] = Array.from({ length: Math.max(1, slot.seats) }, () => []);
    for (const item of held) {
      let seat = seats.find(list => (list.at(-1)?.lastHour ?? -Infinity) < item.firstHour);
      // More holders at once than seats would be a recording oddity; they get a lane of their own rather than overlap.
      if (!seat) { seat = []; seats.push(seat); }
      seat.push(span(item));
    }
    seats.forEach((spans, index) => lanes.push({ id: `${slot.token}#${index}`, label, spans,
      icons: styles.holders !== 'name', names: styles.holders !== 'portrait' }));
  }
  return lanes;
}

/** A law's place among its slot's laws, as "2 of 5", or null for other ideas. */
export function lawLevel(category: CategoryInfo, info: IdeaInfo | undefined): string | null {
  const slot = category.slots.find(item => item.token === info?.slot);
  const level = slot && info ? slot.laws.indexOf(info.token) : -1;
  return slot && level >= 0 ? `${level + 1} of ${slot.laws.length}` : null;
}
