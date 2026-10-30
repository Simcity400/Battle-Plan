import { keyedKnown, runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedRun, KeyedWindow } from '../../dashboard/keyedHistory';
import type { DivisionEquipmentReply, EquipmentTemplate, EquipmentType, EquipmentVariant, KeptReason } from '../../generated/dashboard';

/** One country's prepared division equipment history (`division-equipment-history`). */
export interface DivisionEquipmentHistory {
  branchId: string;
  tag: string;
  countFor: 'owner' | 'host';
  window: KeyedWindow;
  /** The equipment types in the reply's order. */
  kinds: readonly { id: string; archetype: string | null; name: string | null; nameMissing: string | null }[];
  namings: Readonly<Record<string, { name: string | null; nameMissing: KeptReason | null }>>;
  artworks: Readonly<Record<string, { icon: string | null; iconMissing: KeptReason | null }>>;
  designs: Readonly<Record<string, string | null>>;
  /** Each present hour's counted refusals, worded for each hour, and its line for artwork drawn in the recording's style. */
  hours: readonly { firstHour: number; lastHour: number; refusals: readonly { reason: KeptReason; times: number }[]; fallback: string | null }[];
  /** The counted refusals of an hour the country does not exist. */
  absent: readonly string[];
  /** The lines every served reply adds, as of this serving. */
  served: readonly string[];
  warnings: readonly string[];
}

const strings = (value: unknown) => Array.isArray(value) && value.every(item => typeof item === 'string');
const text = (value: unknown) => value === null || typeof value === 'string';
const record = (value: unknown, valid: (item: unknown) => boolean) => !!value && typeof value === 'object' && !Array.isArray(value)
  && Object.values(value).every(valid);
const kept = (value: unknown) => !!value && typeof value === 'object' && strings((value as KeptReason).text)
  && strings((value as KeptReason).tables) && (value as KeptReason).text.length === (value as KeptReason).tables.length + 1;
const keptOrNull = (value: unknown) => value === null || kept(value);

/** Validate a history reply; null is a reply still pending coverage. */
export function validateDivisionEquipmentHistory(value: unknown): DivisionEquipmentHistory | null {
  if (value === null) return null;
  const reply = value as DivisionEquipmentHistory;
  if (!reply || typeof reply !== 'object' || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string'
    || (reply.countFor !== 'owner' && reply.countFor !== 'host')
    || !Array.isArray(reply.kinds) || !reply.kinds.every(kind => kind && typeof kind.id === 'string' && text(kind.archetype) && text(kind.name) && text(kind.nameMissing))
    || !record(reply.namings, item => !!item && text((item as { name: unknown }).name) && keptOrNull((item as { nameMissing: unknown }).nameMissing))
    || !record(reply.artworks, item => !!item && text((item as { icon: unknown }).icon) && keptOrNull((item as { iconMissing: unknown }).iconMissing))
    || !record(reply.designs, text)
    || !Array.isArray(reply.hours) || !reply.hours.every(span => span && Number.isSafeInteger(span.firstHour) && Number.isSafeInteger(span.lastHour)
      && Array.isArray(span.refusals) && span.refusals.every((refusal: { reason: unknown; times: unknown }) => !!refusal && kept(refusal.reason) && Number.isSafeInteger(refusal.times))
      && text(span.fallback))
    || !strings(reply.absent) || !strings(reply.served) || !strings(reply.warnings))
    throw new Error('The division equipment history reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
}

/** A name's value at an hour. */
type Values = ReadonlyMap<string, string>;

function valuesAt(window: KeyedWindow, hour: number): Values {
  const values = new Map<string, string>();
  for (const series of window.series) {
    const run: KeyedRun | undefined = runAtHour(series.runs, hour);
    if (run) values.set(series.name, run[2]);
  }
  return values;
}

/** A kept refusal as a reply at `hour` words it (`KeptReason::at`): each kept table names that hour. */
export function reasonAt(reason: KeptReason, hour: number): string {
  return reason.text.map((part, at) => at === 0 ? part : `hour ${hour} ${reason.tables[at - 1]}${part}`).join('');
}

/** Rust's string order, by code point. */
function codePointOrder(a: string, b: string): number {
  const x = [...a], y = [...b];
  for (let at = 0; at < Math.min(x.length, y.length); at++) {
    const difference = x[at].codePointAt(0)! - y[at].codePointAt(0)!;
    if (difference !== 0) return difference;
  }
  return x.length - y.length;
}

/** Refusals worded for `hour`, then counted and ordered by those words, as the reached product's lines. */
export function refusalLines(refusals: readonly { reason: KeptReason; times: number }[], hour: number): string[] {
  const counted = new Map<string, number>();
  for (const refusal of refusals) {
    if (refusal.times <= 0) continue;
    const words = reasonAt(refusal.reason, hour);
    counted.set(words, (counted.get(words) ?? 0) + refusal.times);
  }
  return [...counted].sort(([a], [b]) => codePointOrder(a, b)).map(([words, times]) => times === 1 ? words : `${words} (${times})`);
}

const handleOrder = (a: string, b: string) => { const x = BigInt(a), y = BigInt(b); return x < y ? -1 : x > y ? 1 : 0; };

/** The reached reply the history holds at `hour`; null where the reached product must answer. */
export function divisionEquipmentAt(history: DivisionEquipmentHistory, hour: number): DivisionEquipmentReply | null {
  if (hour < history.window.firstHour || hour > history.window.lastHour || !keyedKnown(history.window, hour)) return null;
  const values = valuesAt(history.window, hour);
  if (values.has('refused')) return null;
  const value = (name: string) => values.get(name) ?? null;
  const number = (name: string) => { const found = values.get(name); return found === undefined ? null : Number(found); };
  const pool = number('*|pool');
  const reply: DivisionEquipmentReply = { branchId: history.branchId, hour, tag: history.tag, countFor: history.countFor, absent: false,
    poolHour: pool, divisions: 0, divisionsShort: 0, unknownDivisions: 0, types: [], templates: [], warnings: [] };
  if (!values.has('state')) return { ...reply, absent: true, warnings: [...history.absent, ...history.served] };
  // A count the history lacks is not zero: the reached product answers that hour.
  const divisions = number('divisions'), short = number('short'), unknown = number('unknown');
  if (divisions === null || short === null || unknown === null) return null;
  reply.divisions = divisions;
  reply.divisionsShort = short;
  reply.unknownDivisions = unknown;
  // One stockpile read, published once for all countries, dates every stock.
  const stockRead = number('*|stock');
  const types: EquipmentType[] = [];
  for (const kind of history.kinds) {
    const base = `type|${kind.id}|`;
    const held = value(`${base}held`);
    if (held === null) continue;
    const variants: EquipmentVariant[] = [];
    for (const [name, amount] of values) {
      if (!name.startsWith(`${base}variant|`)) continue;
      const variant = name.slice(base.length + 'variant|'.length);
      if (!/^\d+$/.test(variant)) return null;
      const design = value(`${base}design|${variant}`);
      variants.push({ variant, name: design === null ? null : history.designs[design] ?? null, heldRaw: amount });
    }
    variants.sort((a, b) => handleOrder(a.variant, b.variant));
    const stock = value(`${base}stock`);
    const typeShort = number(`${base}short`), typeUnknown = number(`${base}unknown`);
    if (typeShort === null || typeUnknown === null) return null;
    types.push({ archetype: kind.archetype, name: kind.name, nameMissing: kind.nameMissing, heldRaw: held,
      neededRaw: value(`${base}needed`), filledRaw: value(`${base}filled`), missingRaw: value(`${base}missing`),
      divisionsShort: typeShort, unknownDivisions: typeUnknown,
      stockpileRaw: stock, stockpileHour: stock === null ? null : stockRead, variants });
  }
  const templates: EquipmentTemplate[] = [];
  const handles = [...values.keys()].flatMap(name => { const found = /^template\|([^|]*)\|divisions$/.exec(name); return found ? [found[1]] : []; });
  if (handles.some(handle => !/^\d+$/.test(handle))) return null;
  for (const handle of handles.sort(handleOrder)) {
    const base = `template|${handle}|`;
    const templateDivisions = number(`${base}divisions`), templateShort = number(`${base}typesShort`), templateUnknown = number(`${base}unknown`);
    if (templateDivisions === null || templateShort === null || templateUnknown === null) return null;
    const naming = history.namings[value(`${base}naming`) ?? ''];
    const artwork = history.artworks[value(`${base}artwork`) ?? ''];
    const nameMissing = naming?.nameMissing ?? null, iconMissing = artwork?.iconMissing ?? null;
    templates.push({ template: handle, name: naming?.name ?? null, nameMissing: nameMissing === null ? null : reasonAt(nameMissing, hour),
      icon: artwork?.icon ?? null, iconMissing: iconMissing === null ? null : reasonAt(iconMissing, hour), divisions: templateDivisions,
      manpower: value(`${base}manpower`), manpowerRequired: value(`${base}required`), hpRaw: value(`${base}hp`), hpMaxRaw: value(`${base}hpMax`),
      neededRaw: value(`${base}needed`), heldRaw: value(`${base}held`), typesShort: templateShort,
      unknownDivisions: templateUnknown });
  }
  const span = history.hours.find(item => item.firstHour <= hour && hour <= item.lastHour);
  if (!span) return null;
  const lines = refusalLines(span.refusals, hour);
  const rebuilt = { ...reply, types, templates, warnings: [...lines, ...history.served, ...span.fallback === null ? [] : [span.fallback]] };
  // The reached product refuses a reply over its transport budget; so does the history.
  return replyBytes(rebuilt) > REPLY_BUDGET ? null : rebuilt;
}

/** The reached product's reply budget (`MAX_PRODUCT_REPLY_BYTES`). */
export const REPLY_BUDGET = 24 * 1024 * 1024;
const encoder = new TextEncoder();
const replyBytes = (reply: DivisionEquipmentReply) => encoder.encode(JSON.stringify(reply)).length;

/** The history's reply at `hour`, and whether the reached product must be asked instead. */
export function historyAnswer(prepared: boolean, history: { data?: DivisionEquipmentHistory | null; error?: string }, hour: number)
  : { held: DivisionEquipmentReply | null; asked: boolean } {
  const held = prepared && history.data ? divisionEquipmentAt(history.data, hour) : null;
  return { held, asked: !held && (!prepared || history.error !== undefined || history.data !== undefined) };
}

/** One asked range of a country's history; a null end is open. */
export interface HistoryPart { firstHour: number | null; lastHour: number | null; data?: DivisionEquipmentHistory | null; error?: string }

const OVER_BUDGET = /exceeds the transport reply budget/;
export const WHOLE: readonly HistoryPart[] = [{ firstHour: null, lastHour: null }];

export const partAt = (parts: readonly HistoryPart[], hour: number) =>
  parts.find(part => (part.firstHour === null || part.firstHour <= hour) && (part.lastHour === null || hour <= part.lastHour));

/** A range refused as over the reply budget, asked again as its two halves; null when it cannot split. */
export function splitPart(part: HistoryPart, error: string, branch: { firstHour: number; lastHour: number }): HistoryPart[] | null {
  if (!OVER_BUDGET.test(error)) return null;
  const first = part.firstHour ?? branch.firstHour, last = part.lastHour ?? branch.lastHour;
  if (last <= first) return null;
  const middle = first + Math.floor((last - first) / 2);
  return [{ firstHour: part.firstHour, lastHour: middle }, { firstHour: middle + 1, lastHour: part.lastHour }];
}
