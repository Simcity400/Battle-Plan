import { contentTitle } from '../../dashboard/contentTitles.ts';
import { ideaName } from '../../dashboard/ideaHistoryReply.ts';
import type { Artwork, CategoryInfo, IdeaHistoryReply, IdeaInfo, IdeaSpan } from '../../generated/dashboard';

export const UNIDENTIFIED_HOLDER = 'Holder not recorded';

export interface IdeaLaneSpan {
  readonly id: string;
  readonly token: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly label: string;
  readonly icon: string | null;
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

export function lanesFor(reply: IdeaHistoryReply, category: CategoryInfo): IdeaLane[] {
  const infos = new Map<string, IdeaInfo>(reply.ideas.map(info => [info.token, info]));
  const art = new Map<string, Artwork>(reply.artwork.map(item => [item.token, item]));
  const holder = (token: string, character: number, script: string, name: string) => JSON.stringify([token, character, script, name]);
  const holders = new Map<string, Artwork>(reply.holders.map(item => [holder(item.token, item.character, item.characterToken, item.name), item.artwork]));
  const span = (item: IdeaSpan): IdeaLaneSpan => {
    const base = { id: `${item.token}:${item.firstHour}`, token: item.token, firstHour: item.firstHour, lastHour: item.lastHour };
    if (item.character) {
      const look = holders.get(holder(item.token, item.character.id, item.character.token, item.character.name));
      return look ? { ...base, label: look.name ?? 'Holder name unavailable', icon: look.icon ?? null } : { ...base, label: UNIDENTIFIED_HOLDER, icon: null };
    }
    const look = art.get(item.token);
    return { ...base, label: ideaName(look, infos.get(item.token), 'Idea name unavailable'), icon: look?.icon ?? null };
  };
  const lanes: IdeaLane[] = [];
  for (const slot of category.slots) {
    const label = contentTitle(slot.name, slot.token, 'Slot name unavailable');
    const held = reply.spans.filter(item => (item.slot ?? infos.get(item.token)?.slot) === slot.token).sort((a, b) => a.firstHour - b.firstHour);
    if (!slot.character) {
      lanes.push({ id: slot.token, label, slot: slot.token, spans: held.map(item => span(item)) });
      continue;
    }
    const seats: { spans: IdeaLaneSpan[]; last: IdeaSpan | null }[] = Array.from({ length: Math.max(1, slot.seats) }, () => ({ spans: [], last: null }));
    const endings = new Set(held.flatMap(other => other.character ? [`${other.character.id}#${other.lastHour + 1}`] : []));
    const continues = (item: IdeaSpan) => item.character != null && endings.has(`${item.character.id}#${item.firstHour}`);
    const order = held.map(item => ({ item, first: continues(item) ? 0 : 1 }))
      .sort((a, b) => a.item.firstHour - b.item.firstHour || a.first - b.first).map(entry => entry.item);
    for (const item of order) {
      let seat = item.character ? seats.find(({ last }) => last?.character?.id === item.character!.id && last.lastHour + 1 === item.firstHour) : undefined;
      seat ??= seats.find(({ last }) => (last?.lastHour ?? -Infinity) < item.firstHour);
      if (!seat) { seat = { spans: [], last: null }; seats.push(seat); }
      seat.spans.push(span(item));
      seat.last = item;
    }
    seats.forEach(({ spans }, index) => lanes.push({ id: `${slot.token}#${index}`, label, slot: slot.token, spans }));
  }
  return lanes;
}

export function lawLevel(category: CategoryInfo, info: IdeaInfo | undefined): string | null {
  const slot = category.slots.find(item => item.token === info?.slot);
  const level = slot && info ? slot.laws.indexOf(info.token) : -1;
  return slot && level >= 0 ? `${level + 1} of ${slot.laws.length}` : null;
}
