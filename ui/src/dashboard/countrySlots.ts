import type { CountryRef, CountryIdentitiesReply } from '../generated/dashboard';
import type { DashboardClient, ProductRequest } from './client';
import { countryFromKey } from './countryKey.ts';
import type { BoundTrack } from './model';
import { useProduct } from './products.ts';
import type { Failure } from './queryError';
import { productRequest } from './request.ts';

export type { CountryRef } from '../generated/dashboard';

export { countryFromKey, countryKey } from './countryKey.ts';

export function keyNameAt(slots: SlotTags | undefined, key: string, hour: number): string | null {
  return countryNameAt(slots, countryFromKey(key), hour);
}

type Held = { firstHour: number; lastHour: number; countryId: number; tag: string };

export class SlotTags {
  private readonly bySlot = new Map<number, Held[]>();
  private readonly byTag = new Map<string, readonly { firstHour: number; lastHour: number; slot: number }[]>();

  constructor(reply: Pick<CountryIdentitiesReply, 'countries'>) {
    for (const { tag, identities } of reply.countries) this.byTag.set(tag, identities);
    for (const { tag, identities } of reply.countries) for (const run of identities) {
      const held = this.bySlot.get(run.slot);
      const entry = { firstHour: run.firstHour, lastHour: run.lastHour, countryId: run.countryId, tag };
      if (held) held.push(entry); else this.bySlot.set(run.slot, [entry]);
    }
    for (const held of this.bySlot.values()) held.sort((a, b) => a.firstHour - b.firstHour || a.countryId - b.countryId || a.tag.localeCompare(b.tag));
  }

  tagAt(slot: number, hour: number): string | null {
    return this.holderAt(slot, hour)?.tag ?? null;
  }

  private holderAt(slot: number, hour: number): Held | null {
    let found: Held | null = null;
    for (const held of this.bySlot.get(slot) ?? []) {
      if (held.firstHour > hour) break;
      if (held.lastHour >= hour && (!found || held.countryId < found.countryId)) found = held;
    }
    return found;
  }

  runAt(slot: number, hour: number): { tag: string; firstHour: number; lastHour: number } | null {
    const held = this.holderAt(slot, hour);
    return held && { tag: held.tag, firstHour: held.firstHour, lastHour: held.lastHour };
  }

  tagRuns(slot: number): { tag: string; firstHour: number; lastHour: number }[] {
    return (this.bySlot.get(slot) ?? []).map(({ tag, firstHour, lastHour }) => ({ tag, firstHour, lastHour }));
  }

  slotOf(tag: string, hour: number): number | null {
    return this.byTag.get(tag)?.find(run => run.firstHour <= hour && hour <= run.lastHour)?.slot ?? null;
  }

  nameAt(slot: number, hour: number): string | null {
    return this.runNear(slot, hour)?.tag ?? null;
  }

  runNear(slot: number, hour: number): { tag: string; firstHour: number; lastHour: number } | null {
    let best = this.holderAt(slot, hour), distance = Infinity;
    if (!best) for (const held of this.bySlot.get(slot) ?? []) {
      const away = hour < held.firstHour ? held.firstHour - hour : hour - held.lastHour;
      if (away < distance) { best = held; distance = away; }
    }
    return best && { tag: best.tag, firstHour: best.firstHour, lastHour: best.lastHour };
  }
}

const built = new WeakMap<object, SlotTags>();
const slotTagsOf = (reply: CountryIdentitiesReply) => {
  let held = built.get(reply);
  if (!held) { held = new SlotTags(reply); built.set(reply, held); }
  return held;
};

export function useSlotTagsState(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, active = true)
  : { slots: SlotTags | undefined; error: Failure | undefined; retry: () => void } {
  const request: ProductRequest | null = track && active
    ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'country-identities', params: { branchId: track.branch.id } })
    : null;
  const { data, error, retry } = useProduct<CountryIdentitiesReply>(client, request, active && !!track, undefined, track?.source);
  return { slots: data && request ? slotTagsOf(data) : undefined, error: data || !request ? undefined : error, retry };
}

export function useSlotTags(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, active = true): SlotTags | undefined {
  return useSlotTagsState(client, track, active).slots;
}

export function countryTagAt(slots: SlotTags | undefined, country: CountryRef | null | undefined, hour: number): string | null {
  if (country == null) return null;
  if (typeof country === 'string') return country;
  return slots?.tagAt(country, hour) ?? null;
}

export function countryNameAt(slots: SlotTags | undefined, country: CountryRef | null | undefined, hour: number): string | null {
  if (country == null) return null;
  if (typeof country === 'string') return country;
  return slots?.nameAt(country, hour) ?? null;
}
