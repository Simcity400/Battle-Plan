import { contentTitle } from '../../dashboard/contentTitles.ts';
import { holdingLabel, lookOf } from '../../dashboard/ideaHistoryReply.ts';
import type { Artwork, CategoryInfo, IdeaHistoryReply, IdeaHolding, IdeaInfo, IdeaLook } from '../../generated/dashboard';

export interface IdeaLaneSpan {
  readonly id: string;
  readonly holding: IdeaHolding;
  readonly label: string;
  readonly artwork: Artwork;
  readonly look: IdeaLook;
}

export interface IdeaLane {
  readonly id: string;
  readonly label: string;
  readonly slot: string;
  readonly spans: IdeaLaneSpan[];
}

export function timelineCategories(categories: readonly CategoryInfo[]): CategoryInfo[] {
  return categories.filter(category => category.politicsTab && !category.hidden && category.slots.length > 0);
}

export function chosenCategory(categories: readonly CategoryInfo[], chosen: string): CategoryInfo | null {
  const eligible = timelineCategories(categories);
  return chosen ? eligible.find(category => category.token === chosen) ?? null : eligible[0] ?? null;
}

interface Seat { readonly spans: IdeaLaneSpan[]; last: IdeaHolding | null }

export function lanesFor(reply: IdeaHistoryReply, category: CategoryInfo): IdeaLane[] {
  const bySlot = new Map<string, IdeaHolding[]>();
  for (const holding of reply.holdings) {
    const slot = holding.slot ?? lookOf(reply, holding).info.slot;
    if (slot === null) continue;
    const list = bySlot.get(slot);
    if (list) list.push(holding); else bySlot.set(slot, [holding]);
  }
  const span = (holding: IdeaHolding): IdeaLaneSpan => {
    const look = lookOf(reply, holding);
    return { id: `${holding.token}:${holding.firstHour}`, holding, label: holdingLabel(look, holding), artwork: look.artwork, look };
  };
  const slots = new Map<string, { readonly slot: CategoryInfo['slots'][number]; seats: number }>();
  for (const slot of category.slots) {
    const seats = slot.character ? Math.max(1, slot.seats) : 1;
    const held = slots.get(slot.token);
    if (held) held.seats += seats;
    else slots.set(slot.token, { slot, seats });
  }
  const lanes: IdeaLane[] = [];
  for (const { slot, seats: count } of slots.values()) {
    const label = contentTitle(slot.name, slot.token, 'Slot name unavailable');
    const held = bySlot.get(slot.token) ?? [];
    const seats: Seat[] = Array.from({ length: count }, () => ({ spans: [], last: null }));
    const endings = new Set(held.flatMap(other => other.character ? [`${other.character.id}#${other.lastHour + 1}`] : []));
    const continues = (item: IdeaHolding) => item.character != null && endings.has(`${item.character.id}#${item.firstHour}`);
    const order = held.map(item => ({ item, first: continues(item) ? 0 : 1 }))
      .sort((a, b) => a.item.firstHour - b.item.firstHour || a.first - b.first).map(entry => entry.item);
    for (const item of order) {
      let seat = item.character ? seats.find(({ last }) => last?.character?.id === item.character!.id && last.lastHour + 1 === item.firstHour) : undefined;
      seat ??= seats.find(({ last }) => (last?.lastHour ?? -Infinity) < item.firstHour);
      if (!seat) { seat = { spans: [], last: null }; seats.push(seat); }
      seat.spans.push(span(item));
      seat.last = item;
    }
    seats.forEach(({ spans }, index) => lanes.push({ id: slot.character || index > 0 ? `${slot.token}#${index}` : slot.token, label, slot: slot.token, spans }));
  }
  return lanes;
}

export function lawLevel(category: CategoryInfo, info: IdeaInfo | undefined): string | null {
  const slot = category.slots.find(item => item.token === info?.slot);
  const level = slot && info ? slot.laws.indexOf(info.token) : -1;
  return slot && level >= 0 ? `${level + 1} of ${slot.laws.length}` : null;
}
