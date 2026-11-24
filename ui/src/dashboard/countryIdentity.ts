import { countryFromKey } from './countryKey.ts';
import { runAt } from '../charts/series.ts';
import type { CountryIdentityHistory, CountryIdentityRun, MembershipRun } from '../generated/dashboard';
import type { Span } from '../charts/series';

export const countryIdentityKey = (identity: Pick<CountryIdentityRun, 'slot' | 'countryId'>): string => JSON.stringify([identity.slot, identity.countryId]);

export function validCountryIdentityRuns(runs: unknown): boolean {
  return runs === undefined || Array.isArray(runs) && runs.every((run, index) => run != null
    && Number.isInteger(run.firstHour) && Number.isInteger(run.lastHour) && run.firstHour <= run.lastHour
    && run.firstHour >= -0x8000_0000 && run.lastHour <= 0x7fff_ffff
    && Number.isInteger(run.slot) && run.slot >= 0 && run.slot <= 0xffff_ffff
    && Number.isInteger(run.countryId) && run.countryId >= 0 && run.countryId <= 0xffff_ffff
    && (index === 0 || runs[index - 1].lastHour < run.firstHour));
}

export function validCountryIdentityHistories(value: unknown): value is CountryIdentityHistory[] {
  if (!Array.isArray(value)) return false;
  const tags = new Set<string>();
  for (const history of value) {
    if (history == null || typeof history.tag !== 'string' || tags.has(history.tag)
      || !Array.isArray(history.identities) || !validCountryIdentityRuns(history.identities)) return false;
    tags.add(history.tag);
  }
  return true;
}

type IdentityHistory = { tag: string; identities: readonly CountryIdentityRun[] };
type KeyRun = Span & { key: string };

export class CountryIdentityIndex {
  readonly changeHours: ReadonlyArray<number>;
  private readonly byTag = new Map<string, KeyRun[]>();
  private readonly byKey = new Map<string, Map<string, CountryIdentityRun[]>>();
  private readonly bySlot = new Map<number, Set<string>>();
  private readonly order = new Map<string, number>();
  private readonly choices = new WeakMap<readonly string[], ReadonlySet<string>>();

  constructor(histories: readonly IdentityHistory[]) {
    if (!validCountryIdentityHistories(histories)) throw new Error('The native country identity catalogue is malformed.');
    const hours = new Set<number>();
    for (const history of histories) {
      this.order.set(history.tag, this.order.size);
      this.byTag.set(history.tag, history.identities.map(identity => {
        hours.add(identity.firstHour); hours.add(identity.lastHour + 1);
        const key = countryIdentityKey(identity);
        const slotKeys = this.bySlot.get(identity.slot);
        if (slotKeys) slotKeys.add(key); else this.bySlot.set(identity.slot, new Set([key]));
        let aliases = this.byKey.get(key);
        if (!aliases) { aliases = new Map(); this.byKey.set(key, aliases); }
        const runs = aliases.get(history.tag);
        if (runs) runs.push(identity); else aliases.set(history.tag, [identity]);
        return { firstHour: identity.firstHour, lastHour: identity.lastHour, key };
      }));
    }
    this.changeHours = [...hours].sort((a, b) => a - b);
  }

  keyAt(tag: string, hour: number): string | null {
    const slot = countryFromKey(tag);
    if (typeof slot === 'number') {
      for (const key of this.bySlot.get(slot) ?? []) for (const runs of this.byKey.get(key)?.values() ?? []) if (runAt(runs, hour)) return key;
      return null;
    }
    return runAt(this.byTag.get(tag) ?? [], hour)?.key ?? null;
  }

  selected(chosen: readonly string[]): ReadonlySet<string> {
    let keys = this.choices.get(chosen);
    if (!keys) {
      const selected = new Set<string>();
      for (const tag of chosen) {
        const slot = countryFromKey(tag);
        if (typeof slot === 'number') { for (const key of this.bySlot.get(slot) ?? []) selected.add(key); continue; }
        for (const run of this.byTag.get(tag) ?? []) selected.add(run.key);
      }
      keys = selected;
      this.choices.set(chosen, keys);
    }
    return keys;
  }

  changeHoursOf(chosen: readonly string[]): number[] {
    const hours = new Set<number>();
    const add = (run: Span) => { hours.add(run.firstHour); hours.add(run.lastHour + 1); };
    for (const tag of chosen) for (const run of this.byTag.get(tag) ?? []) add(run);
    for (const key of this.selected(chosen)) for (const runs of this.byKey.get(key)?.values() ?? []) for (const run of runs) add(run);
    return [...hours].sort((a, b) => a - b);
  }

  identityOf(key: string): { slot: number; countryId: number } {
    const [slot, countryId] = JSON.parse(key) as [number, number];
    return { slot, countryId };
  }

  keysDuring(memberships: readonly MembershipRun[]): ReadonlySet<string> {
    const keys = new Set<string>();
    for (const member of memberships) for (const run of this.runsDuring(member)) keys.add(run.key);
    return keys;
  }

  firstHoursDuring(memberships: readonly MembershipRun[]): ReadonlyMap<string, number> {
    const first = new Map<string, number>();
    for (const member of memberships) for (const run of this.runsDuring(member)) {
      const hour = Math.max(member.firstHour, run.firstHour), known = first.get(run.key);
      if (known === undefined || hour < known) first.set(run.key, hour);
    }
    return first;
  }

  membershipRuns(memberships: readonly MembershipRun[], firstHour = -0x8000_0000, lastHour = 0x7fff_ffff): MembershipRun[] {
    const runs: MembershipRun[] = [];
    for (const member of memberships) for (const run of this.runsDuring(member)) {
      const first = Math.max(firstHour, member.firstHour, run.firstHour), last = Math.min(lastHour, member.lastHour, run.lastHour);
      if (first <= last) runs.push({ tag: member.tag, firstHour: first, lastHour: last });
    }
    return runs;
  }

  private *runsDuring(member: MembershipRun): Generator<KeyRun> {
    const runs = this.byTag.get(member.tag) ?? [];
    let low = 0, high = runs.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (runs[middle].lastHour < member.firstHour) low = middle + 1; else high = middle;
    }
    for (let index = low; index < runs.length && runs[index].firstHour <= member.lastHour; index++) yield runs[index];
  }

  aliasesAt(keys: ReadonlySet<string>, hour: number): string[] {
    const tags = new Set<string>();
    for (const key of keys) for (const [tag, runs] of this.byKey.get(key) ?? []) if (runAt(runs, hour)) tags.add(tag);
    return [...tags].sort((a, b) => this.order.get(a)! - this.order.get(b)!);
  }

  holderAliases(tag: string, slot: number): MembershipRun[] | null {
    const keys = new Set((this.byTag.get(tag) ?? []).map(run => run.key)
      .filter(key => (JSON.parse(key) as [number, number])[0] === slot));
    return keys.size === 1 ? this.aliasRuns(keys, -0x8000_0000, 0x7fff_ffff) : null;
  }

  aliasRuns(keys: ReadonlySet<string>, firstHour: number, lastHour: number): MembershipRun[] {
    const aliases: MembershipRun[] = [];
    for (const key of keys) for (const [tag, runs] of this.byKey.get(key) ?? []) for (const run of runs) {
      const first = Math.max(firstHour, run.firstHour), last = Math.min(lastHour, run.lastHour);
      if (first <= last) aliases.push({ tag, firstHour: first, lastHour: last });
    }
    return aliases.sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  }
}

const indexes = new WeakMap<readonly IdentityHistory[], CountryIdentityIndex>();
export function countryIdentityIndex(histories: readonly IdentityHistory[]): CountryIdentityIndex {
  let index = indexes.get(histories);
  if (!index) { index = new CountryIdentityIndex(histories); indexes.set(histories, index); }
  return index;
}

export interface CountryDrillHistory { tag: string; identities?: readonly CountryIdentityRun[]; segments?: readonly Span[] }

export function historyAt<H extends { tag: string; identities?: readonly CountryIdentityRun[] }>(histories: readonly H[], key: string, hour: number): H | undefined {
  const slot = countryFromKey(key);
  return typeof slot === 'number' ? histories.find(history => runAt(history.identities ?? [], hour)?.slot === slot) : histories.find(history => history.tag === key);
}

export function countryDrillSpans(histories: readonly CountryDrillHistory[], tag: string, originHour: number): ReadonlyMap<string, readonly Span[]> {
  const origin = historyAt(histories, tag, originHour);
  const identity = runAt(origin?.identities ?? [], originHour);
  if (!identity) {
    const segment = runAt(origin?.segments ?? [], originHour);
    return new Map<string, readonly Span[]>(origin?.identities === undefined && segment ? [[tag, [segment]]] : []);
  }
  const key = countryIdentityKey(identity);
  return new Map<string, readonly Span[]>(histories.map(history => [history.tag,
    (history.identities ?? []).filter(identity => countryIdentityKey(identity) === key)] as const));
}

export function countryDrillTagAt(byTag: ReadonlyMap<string, readonly Span[]>, hour: number): string | null {
  let found: string | null = null;
  for (const [tag, spans] of byTag) {
    if (!runAt(spans, hour)) continue;
    if (found !== null) return null;
    found = tag;
  }
  return found;
}
