import type { CountryRef, CountryIdentitiesReply } from '../generated/dashboard';
import type { DashboardClient, ProductRequest } from './client';
import { countryFromKey } from './countryKey.ts';
import type { BoundTrack } from './model';
import { useProduct } from './products.ts';

export type { CountryRef } from '../generated/dashboard';

export { countryFromKey, countryKey } from './countryKey.ts';

/** The tag a [`countryKey`] goes by at `hour`, for a label, colour or flag: a tag key's own tag, or its slot's tag
 * then (else the nearest it held); null where that is not known. */
export function keyNameAt(slots: SlotTags | undefined, key: string, hour: number): string | null {
  return countryNameAt(slots, countryFromKey(key), hour);
}

type Held = { firstHour: number; lastHour: number; countryId: number; tag: string };

/** Each canonical slot's tags over a history, from its `country-identities` reply: the tag a slot's country goes by at
 * an hour is the one its holder (the lowest raw id the slot holds a tag under then) holds, as the service resolves it. */
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

  /** The tag `slot`'s country goes by at `hour`; null at an hour it holds no tag. */
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

  /** The run of the tag `slot`'s country goes by at `hour` ([`tagAt`]'s); null at an hour it holds no tag. */
  runAt(slot: number, hour: number): { tag: string; firstHour: number; lastHour: number } | null {
    const held = this.holderAt(slot, hour);
    return held && { tag: held.tag, firstHour: held.firstHour, lastHour: held.lastHour };
  }

  /** Each tag `slot`'s country went by, with the hours it did, in hour order. */
  tagRuns(slot: number): { tag: string; firstHour: number; lastHour: number }[] {
    return (this.bySlot.get(slot) ?? []).map(({ tag, firstHour, lastHour }) => ({ tag, firstHour, lastHour }));
  }

  /** The slot of the country holding `tag` at `hour`; null where none does. */
  slotOf(tag: string, hour: number): number | null {
    return this.byTag.get(tag)?.find(run => run.firstHour <= hour && hour <= run.lastHour)?.slot ?? null;
  }

  /** The tag to name `slot`'s country by near `hour`: its tag then, else the nearest it held; null for a slot the
   * history never holds. */
  nameAt(slot: number, hour: number): string | null {
    return this.runNear(slot, hour)?.tag ?? null;
  }

  /** The run [`nameAt`] names `slot` by near `hour`: its holder's then, else the nearest; null for a slot the
   * history never holds. */
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

/** The track's slot tags, once its history's identities are read; undefined until then. */
export function useSlotTags(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, active = true): SlotTags | undefined {
  const request: ProductRequest | null = track
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-identities', params: { branchId: track.branch.id } }
    : null;
  const data = useProduct<CountryIdentitiesReply>(client, request, active && !!track, undefined, track?.source).data;
  return data ? slotTagsOf(data) : undefined;
}

/** The tag `country` goes by at `hour`: a tag choice's own tag, or its slot's tag then; null where that is not known
 * (a slot holding no tag then, or identities not yet read). */
export function countryTagAt(slots: SlotTags | undefined, country: CountryRef | null | undefined, hour: number): string | null {
  if (country == null) return null;
  if (typeof country === 'string') return country;
  return slots?.tagAt(country, hour) ?? null;
}

/** The tag to name `country` by near `hour`, for a label: as [`countryTagAt`], else the nearest tag its slot held. */
export function countryNameAt(slots: SlotTags | undefined, country: CountryRef | null | undefined, hour: number): string | null {
  if (country == null) return null;
  if (typeof country === 'string') return country;
  return slots?.nameAt(country, hour) ?? null;
}
