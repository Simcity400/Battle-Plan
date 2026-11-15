/** The air ledger's windows joined into the ledger a widget shows (`crates/dashboard/src/products/air_ledger/reference.rs`,
 * `join`, proves the windows joined from the first counted equal to the whole-branch reply they replaced): spans and the
 * service's top N memberships laid end to end, each part's totals carried on from the windows before it, names and notes
 * once each. A join appends each window as it lands, its cost that window's, never the windows already held. */
import type { HourSpan, MilitaryAirCountry, MilitaryAirName, MilitaryAirPart, MilitaryAirReply } from '../generated/dashboard';
import { WINDOW_HOURS, windowsOf } from './heldWindows.ts';
import { keepLedgerTotals, ledgerTotals, openingTotals } from './militaryAirLedger.ts';
import type { LedgerTotal, MilitaryAirLedger } from './militaryAirLedger';

/** How far a widget's ledger reaches: the hour it shows (a race of totals: its window, which carries the totals before
 * it, and following the next), or its whole span (a view over the span, a breakdown with its final values, a drill
 * opening at once). */
export type LedgerReach = 'shown' | 'end';

/** The windows a widget showing `shown` of `start..=end` wants: the window shown and, following, the next ahead, never
 * past the span; or, reaching the end, every window of the span. */
export function ledgerWindows(start: number, end: number, shown: number, reach: LedgerReach, following: boolean): number[] {
  if (reach === 'end') return windowsOf(start, end);
  const at = Math.min(end, Math.max(start, shown));
  return windowsOf(at, following ? Math.min(end, at + WINDOW_HOURS) : at);
}

/** `span` after `spans`, joined to the last where it meets or overlaps it: a window's hours reaching back to where they
 * began overlap the windows before. */
function pushJoined(spans: HourSpan[], span: HourSpan) {
  const last = spans[spans.length - 1];
  if (last && span.firstHour <= last.lastHour + 1) spans[spans.length - 1] = { firstHour: Math.min(last.firstHour, span.firstHour), lastHour: Math.max(last.lastHour, span.lastHour) };
  else spans.push({ firstHour: span.firstHour, lastHour: span.lastHour });
}

const compareText = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
/** None before any value, as the service orders its parts. */
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

/** A total of at most fifteen digits, which a number holds exactly. */
const SAFE_TOTAL = /^-?\d{1,15}$/;
/** `total` carried on by `base`, exactly. */
function carried(base: string, total: string): string {
  if (base === '0') return total;
  if (SAFE_TOTAL.test(base) && SAFE_TOTAL.test(total)) {
    const sum = Number(base) + Number(total);
    if (Number.isSafeInteger(sum)) return String(sum);
  }
  return String(BigInt(base) + BigInt(total));
}

/** Consecutive windows from `first`, joined as each is appended. The arrays it hands out grow in place as later windows
 * join; a ledger it gave stays right for the hours it covered. */
export class LedgerJoin {
  /** The last window joined; the one before `first` while none is. */
  last: number;
  private head: MilitaryAirReply | null = null;
  private tail: MilitaryAirReply | null = null;
  private readonly observed: HourSpan[] = [];
  private readonly incomplete: HourSpan[] = [];
  private readonly holders = new Map<string, MilitaryAirCountry>();
  private readonly parts = new Map<string, { part: MilitaryAirPart; carried: string }>();
  private readonly regions = new Map<number, MilitaryAirName>();
  private readonly buildings = new Map<string, MilitaryAirName>();
  private readonly notes = new Map<string, number>();
  private readonly warnings: string[] = [];
  private readonly warned = new Set<string>();
  /** Each holder identity's totals (null: the unattributed), the first what the first window carried in. */
  private readonly totals = new Map<string | null, LedgerTotal[]>();
  private given: MilitaryAirLedger | null = null;

  /** What names the windows' request. */
  readonly key: string;
  /** The window the join starts with. */
  readonly first: number;

  constructor(key: string, first: number) {
    this.key = key;
    this.first = first;
    this.last = first - 1;
  }

  /** The window to append next. */
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
    for (const note of window.notes) this.notes.set(note.reason, (this.notes.get(note.reason) ?? 0) + note.events);
    for (const warning of window.warnings) if (!this.warned.has(warning)) { this.warned.add(warning); this.warnings.push(warning); }
  }

  /** The windows joined so far; none before one with hours is. */
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
      warnings: [...[...this.notes].sort(([a], [b]) => compareText(a, b)).map(([reason, events]) => `${events} events: ${reason}`), ...this.warnings],
    };
    keepLedgerTotals(ledger, this.totals);
    this.given = ledger;
    return ledger;
  }
}

/** `windows`, consecutive and ascending, joined. */
export function joinMilitaryAir(windows: readonly MilitaryAirReply[]): MilitaryAirLedger | null {
  const join = new LedgerJoin('', windows[0]?.window ?? 0);
  for (const window of windows) join.append(window);
  return join.ledger();
}
