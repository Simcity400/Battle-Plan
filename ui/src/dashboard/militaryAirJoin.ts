import type { HourSpan, MilitaryAirCountry, MilitaryAirName, MilitaryAirCauseName, MilitaryAirPart, MilitaryAirReply } from '../generated/dashboard';
import { WINDOW_HOURS, windowsOf } from './heldWindows.ts';
import { keepLedgerTotals, ledgerTotals, openingTotals } from './militaryAirLedger.ts';
import type { LedgerTotal, MilitaryAirLedger } from './militaryAirLedger';

export type LedgerReach = 'shown' | 'end';

export function ledgerWindows(start: number, end: number, shown: number, reach: LedgerReach, following: boolean): number[] {
  if (reach === 'end') return windowsOf(start, end);
  const at = Math.min(end, Math.max(start, shown));
  return windowsOf(at, following ? Math.min(end, at + WINDOW_HOURS) : at);
}

function pushJoined(spans: HourSpan[], span: HourSpan) {
  const last = spans[spans.length - 1];
  if (last && span.firstHour <= last.lastHour + 1) spans[spans.length - 1] = { firstHour: Math.min(last.firstHour, span.firstHour), lastHour: Math.max(last.lastHour, span.lastHour) };
  else spans.push({ firstHour: span.firstHour, lastHour: span.lastHour });
}

const compareText = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
function compareOptional<T extends string | number | boolean>(a: T | null, b: T | null): number {
  if (a === b) return 0;
  if (a === null) return -1;
  if (b === null) return 1;
  return a < b ? -1 : 1;
}
const comparePart = (a: MilitaryAirPart, b: MilitaryAirPart) => compareOptional(a.tag, b.tag) || compareOptional(a.identity, b.identity)
  || compareOptional(a.dealt, b.dealt) || compareOptional(a.cause, b.cause) || compareOptional(a.opponentKnown, b.opponentKnown)
  || compareOptional(a.buildingToken, b.buildingToken) || compareOptional(a.buildingId, b.buildingId) || compareOptional(a.state, b.state);
const partKey = (part: MilitaryAirPart) => JSON.stringify([part.tag, part.identity, part.dealt, part.cause, part.opponentKnown, part.buildingToken, part.buildingId, part.state]);

const SAFE_TOTAL = /^-?\d{1,15}$/;
function carried(base: string, total: string): string {
  if (base === '0') return total;
  if (SAFE_TOTAL.test(base) && SAFE_TOTAL.test(total)) {
    const sum = Number(base) + Number(total);
    if (Number.isSafeInteger(sum)) return String(sum);
  }
  return String(BigInt(base) + BigInt(total));
}

export class LedgerJoin {
  last: number;
  private head: MilitaryAirReply | null = null;
  private tail: MilitaryAirReply | null = null;
  private readonly observed: HourSpan[] = [];
  private readonly incomplete: HourSpan[] = [];
  private readonly holders = new Map<string, MilitaryAirCountry>();
  private readonly parts = new Map<string, { part: MilitaryAirPart; carried: string }>();
  private readonly regions = new Map<number, MilitaryAirName>();
  private readonly buildings = new Map<string, MilitaryAirName>();
  private readonly causes = new Map<number, MilitaryAirCauseName>();
  private readonly notes = new Map<string, number>();
  private readonly warnings: string[] = [];
  private readonly warned = new Set<string>();
  private readonly totals = new Map<string | null, LedgerTotal[]>();
  private given: MilitaryAirLedger | null = null;

  readonly key: string;
  readonly first: number;

  constructor(key: string, first: number) {
    this.key = key;
    this.first = first;
    this.last = first - 1;
  }

  get next() { return this.last + 1; }

  append(window: MilitaryAirReply) {
    this.last++;
    this.given = null;
    if (window.firstHour > window.lastHour) return;
    if (!this.head) {
      this.head = window;
      for (const [identity, carried] of openingTotals(window.openings)) {
        if (carried.dealt !== 0n || carried.taken !== 0n) this.totals.set(identity, [{ readHour: window.firstHour - 1, nativeHour: null, ...carried }]);
      }
    }
    this.tail = window;
    for (const span of window.observed) pushJoined(this.observed, span);
    for (const span of window.incomplete) pushJoined(this.incomplete, span);
    for (const country of window.countries) {
      const key = JSON.stringify([country.tag, country.identity]);
      const held = this.holders.get(key) ?? { tag: country.tag, identity: country.identity, present: [], membership: null };
      this.holders.set(key, held);
      for (const span of country.present) pushJoined(held.present, span);
      if (country.membership) {
        held.membership ??= [];
        for (const span of country.membership) pushJoined(held.membership, span);
      }
    }
    const byHolder = new Map<string | null, MilitaryAirPart[]>();
    for (const part of window.parts) {
      const key = partKey(part), held = this.parts.get(key);
      byHolder.set(part.identity, [...(byHolder.get(part.identity) ?? []), part]);
      if (!held) {
        this.parts.set(key, { part: { ...part, points: [...part.points] }, carried: part.points[part.points.length - 1]?.[2] ?? '0' });
        continue;
      }
      if (!part.points.length) continue;
      const base = held.carried;
      for (const [hour, readHour, total] of part.points) held.part.points.push([hour, readHour, carried(base, total)]);
      held.carried = held.part.points[held.part.points.length - 1]![2];
    }
    for (const [identity, parts] of byHolder) {
      const local = ledgerTotals(parts);
      if (!local.length) continue;
      const held = this.totals.get(identity) ?? [], before = held[held.length - 1];
      for (const total of local) held.push(before ? { ...total, dealt: total.dealt + before.dealt, taken: total.taken + before.taken } : total);
      this.totals.set(identity, held);
    }
    for (const name of window.regions) if (!this.regions.has(Number(name.id))) this.regions.set(Number(name.id), name);
    for (const name of window.buildings) if (!this.buildings.has(name.id)) this.buildings.set(name.id, name);
    for (const cause of window.causes ?? []) if (!this.causes.has(cause.cause)) this.causes.set(cause.cause, cause);
    for (const note of window.notes) this.notes.set(note.reason, (this.notes.get(note.reason) ?? 0) + note.events);
    for (const warning of window.warnings) if (!this.warned.has(warning)) { this.warned.add(warning); this.warnings.push(warning); }
  }

  ledger(): MilitaryAirLedger | null {
    if (this.given || !this.head || !this.tail) return this.given;
    const head = this.head;
    const countries = [...this.holders.values()].sort((a, b) => compareText(a.tag, b.tag) || compareText(a.identity, b.identity))
      .map(holder => ({ ...holder, present: [...holder.present], membership: holder.membership && [...holder.membership] }));
    const ledger: MilitaryAirLedger = {
      family: head.family, firstHour: head.firstHour, lastHour: this.tail.lastHour, countedFrom: head.countedFrom,
      available: this.observed.length > 0, observed: [...this.observed], incomplete: [...this.incomplete], countries, openings: head.openings,
      parts: [...this.parts.values()].map(held => held.part).sort(comparePart),
      regions: [...this.regions].sort(([a], [b]) => a - b).map(([, name]) => name),
      buildings: [...this.buildings].sort(([a], [b]) => compareText(a, b)).map(([, name]) => name),
      causes: [...this.causes].sort(([a], [b]) => a - b).map(([, cause]) => cause),
      warnings: [...[...this.notes].sort(([a], [b]) => compareText(a, b)).map(([reason, events]) => `${events} events: ${reason}`), ...this.warnings],
    };
    keepLedgerTotals(ledger, this.totals);
    this.given = ledger;
    return ledger;
  }
}

