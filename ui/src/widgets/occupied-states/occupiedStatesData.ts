import { runAt, spansCovered } from '../../charts/series.ts';
import type { Span } from '../../charts/series.ts';
import { formatAmount, formatFixed, formatNativeFixed5, formatSigned } from '../../charts/valueAxis.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { fixed5At, validateFixed5Runs } from '../../dashboard/fixed5Runs.ts';
import { artworkUrlValue } from '../../dashboard/payload.ts';
import type { NativeFixed5Run } from '../../dashboard/fixed5Runs.ts';
import type { HourSpan, OccupiedObservation, OccupiedSeries, OccupiedStatesManifestReply, OccupiedStatesReply } from '../../generated/dashboard';

const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const hour = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= -2147483648 && v <= 2147483647;
const nullable = (v: unknown) => v === null || typeof v === 'string';
const raw = (v: unknown) => typeof v === 'string' && /^-?\d{1,19}$/.test(v) && BigInt(v) >= -(1n << 63n) && BigInt(v) < (1n << 63n);
const tag = (v: unknown) => v === null || (typeof v === 'string' && v.length > 0 && !v.includes('\0'));
const strings = (v: unknown) => Array.isArray(v) && v.every(item => typeof item === 'string');
const span = (v: unknown): v is { firstHour: number; lastHour: number } => object(v) && hour(v.firstHour) && hour(v.lastHour) && v.firstHour <= v.lastHour;
const ordered = (v: unknown): v is (Record<string, unknown> & { firstHour: number; lastHour: number })[] => Array.isArray(v) && v.every((item, i) => span(item) && (!i || v[i - 1].lastHour < item.firstHour));
/** Only an observation starting at the window can continue an earlier read. */
const observations = (v: unknown, interval: HourSpan) => ordered(v) && v.every((item, index) => {
  const previous = v[index - 1];
  return hour(item.readHour) && item.readHour <= item.firstHour && typeof item.complete === 'boolean'
    && item.firstHour >= interval.firstHour && item.lastHour <= interval.lastHour
    && (!previous || Number(previous.readHour) <= item.readHour)
    && (item.readHour === item.firstHour || (previous ? previous.lastHour === item.firstHour - 1 : item.firstHour === interval.firstHour));
});
/** Ordered changes inside the spans, each span restating the value at its first hour,
 * so none carries across a gap or from another window. */
function changes(v: unknown, value: (v: unknown) => boolean, spans: readonly Span[]): boolean {
  if (!Array.isArray(v) || !v.every((change, i) => Array.isArray(change) && change.length === 2 && hour(change[0]) && value(change[1])
    && (!i || v[i - 1][0] < change[0]))) return false;
  let next = 0;
  for (const span of spans) {
    if (v[next]?.[0] !== span.firstHour) return false;
    while (next < v.length && v[next][0] <= span.lastHour) next++;
  }
  return next === v.length;
}
/** Every fixed5 value is known over the same spans as resistance. */
const fixed5Spans = (v: unknown, spans: readonly Span[]) => validateFixed5Runs(v, 0) && v.length === spans.length
  && v.every((run, i) => run.firstHour === spans[i].firstHour && run.lastHour === spans[i].lastHour);

/** Law facts, each token once: a manifest's, or those a window answered
 * before preparation reached it carries for its states. */
const lawList = (laws: unknown) => Array.isArray(laws) && laws.every(law => object(law) && typeof law.token === 'string' && nullable(law.name) && nullable(law.missing)
  && (law.icon === null || artworkUrlValue(law.icon)))
  && new Set(laws.map(law => (law as { token: string }).token)).size === laws.length;

export function validateOccupiedStatesManifest(data: unknown): OccupiedStatesManifestReply {
  if (!object(data) || typeof data.branchId !== 'string' || !/^\d+$/.test(data.branchId) || data.decimals !== 5 || !ordered(data.windows)
    || !lawList(data.laws) || typeof data.strengthMissing !== 'string' || !strings(data.warnings)) {
    throw new Error('The occupied states reply is malformed.');
  }
  return data as unknown as OccupiedStatesManifestReply;
}

export function validateOccupiedStatesReply(data: unknown): OccupiedStatesReply {
  const malformed = new Error('The occupied states window is malformed.');
  if (!object(data) || typeof data.branchId !== 'string' || !/^\d+$/.test(data.branchId) || !Number.isSafeInteger(data.window) || Number(data.window) < 0
    || !span(data.interval) || !observations(data.observed, data.interval) || !strings(data.texts) || !strings(data.warnings) || !Array.isArray(data.states)
    || data.laws !== undefined && !lawList(data.laws)) throw malformed;
  const reply = data as unknown as OccupiedStatesReply;
  const { interval, texts } = reply;
  const index = (check: (text: string) => boolean) => (v: unknown) => v === null || (Number.isInteger(v) && Number(v) >= 0 && Number(v) < texts.length && check(texts[Number(v)]));
  const text = index(() => true), country = index(tag);
  const nullableRaw = (v: unknown) => v === null || raw(v), nullableInt = (v: unknown) => v === null || hour(v);
  const values: [keyof OccupiedSeries, (v: unknown) => boolean][] = [['name', text], ['nameMissing', text], ['owner', country], ['occupier', country],
    ['occupied', country], ['garrisonStrengthRaw', nullableRaw], ['requiredManpower', nullableInt], ['garrisonRequiredRaw', nullableRaw],
    ['garrisonAlert', v => typeof v === 'boolean'], ['garrisonRed', v => v === null || typeof v === 'boolean'], ['assignedManpower', nullableInt],
    ['garrisonMissing', text], ['law', text], ['lawMissing', text]];
  let previous = -1;
  for (const series of data.states) {
    if (!object(series) || !Number.isInteger(series.state) || Number(series.state) <= previous || Number(series.state) > 4294967295) throw malformed;
    previous = Number(series.state);
    const spans = series.resistance;
    // Resistance runs are the hours the state is occupied, inside the window and its observations.
    if (!validateFixed5Runs(spans, 0) || !spans.length || spans[0].firstHour < interval.firstHour || spans[spans.length - 1].lastHour > interval.lastHour
      || !spansCovered(spans, reply.observed) || !['compliance', 'resistanceSpeed', 'complianceSpeed'].every(key => fixed5Spans(series[key], spans))
      || !values.every(([key, value]) => changes(series[key], value, spans))) throw malformed;
  }
  return reply;
}

/** A state's values at one hour, decoded from its window. Raw values are exact
 * signed decimal strings: fixed5 percentage units (speeds per day) and fractions. */
export interface OccupiedValues {
  name: string | null; nameMissing: string | null; owner: string | null; occupier: string | null;
  /** Original occupied country, which can differ from today's legal owner. */
  occupied: string | null;
  resistanceRaw: string; complianceRaw: string; resistanceSpeedRaw: string; complianceSpeedRaw: string;
  garrisonStrengthRaw: string | null; requiredManpower: number | null; garrisonRequiredRaw: string | null;
  garrisonAlert: boolean; garrisonRed: boolean | null; assignedManpower: number | null;
  garrisonMissing: string | null; law: string | null; lawMissing: string | null;
}

/** The last change at or before `hour`; validation put one at the start of its span. */
function changeAt<T>(changes: readonly (readonly [number, T])[], hour: number): T {
  let low = 0;
  let high = changes.length - 1;
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if (changes[middle][0] <= hour) low = middle; else high = middle - 1;
  }
  return changes[low][1];
}

/** A state's values at `hour`, or null outside the spans it is occupied. */
export function valuesAt(reply: OccupiedStatesReply, series: OccupiedSeries, hour: number): OccupiedValues | null {
  const resistance = fixed5At(series.resistance, hour);
  if (!resistance) return null;
  const fixed5 = (runs: readonly NativeFixed5Run[]) => fixed5At(runs, hour)!.raw!;
  const text = (changes: readonly (readonly [number, number | null])[]) => {
    const index = changeAt(changes, hour);
    return index === null ? null : reply.texts[index];
  };
  return { name: text(series.name), nameMissing: text(series.nameMissing), owner: text(series.owner), occupier: text(series.occupier), occupied: text(series.occupied),
    resistanceRaw: resistance.raw!, complianceRaw: fixed5(series.compliance), resistanceSpeedRaw: fixed5(series.resistanceSpeed), complianceSpeedRaw: fixed5(series.complianceSpeed),
    garrisonStrengthRaw: changeAt(series.garrisonStrengthRaw, hour), requiredManpower: changeAt(series.requiredManpower, hour),
    garrisonRequiredRaw: changeAt(series.garrisonRequiredRaw, hour), garrisonAlert: changeAt(series.garrisonAlert, hour), garrisonRed: changeAt(series.garrisonRed, hour),
    assignedManpower: changeAt(series.assignedManpower, hour), garrisonMissing: text(series.garrisonMissing), law: text(series.law), lawMissing: text(series.lawMissing) };
}

/** The window holding `hour` (the last starting at or before it; past its end is a
 * gap) and, while following playback, both neighbours, so playing or scrubbing into
 * either shows at once. */
export function occupiedWindows(windows: readonly HourSpan[], hour: number, follow: boolean): { target: number; wanted: number[] } {
  let target = windows.length - 1;
  while (target >= 0 && windows[target].firstHour > hour) target--;
  const wanted = target < 0 ? [] : follow ? [target - 1, target, target + 1].filter(index => index >= 0 && index < windows.length) : [target];
  return { target, wanted };
}

export interface OccupiedRow { key: string; state: number; hour: number; value: OccupiedValues }
export const stateName = (row: OccupiedRow) => row.value.name ?? `State ${row.state}`;
/** Owner follows hourly States; occupation endpoints belong to the daily read. */
export const countryHour = (row: OccupiedRow, role: 'owner' | 'occupier' | 'occupied', shownHour: number) => role === 'owner' ? shownHour : row.hour;
export function occupiedAt(reply: OccupiedStatesReply, hour: number, matches: ((tag: string, readHour: number) => boolean) | null, role: 'occupier' | 'occupied',
  identityKnown?: (tag: string, readHour: number) => boolean): { observation: OccupiedObservation | null; rows: OccupiedRow[]; selectionUnknown: boolean } {
  const observation = runAt(reply.observed, hour);
  if (!observation) return { observation: null, rows: [], selectionUnknown: false };
  const rows: OccupiedRow[] = [];
  let selectionUnknown = false;
  for (const series of reply.states) {
    const value = valuesAt(reply, series, hour);
    if (!value) continue;
    if (matches) {
      const tag = value[role];
      if (tag === null || identityKnown && !identityKnown(tag, observation.readHour)) { selectionUnknown = true; continue; }
      if (!matches(tag, observation.readHour)) continue;
    }
    rows.push({ key: String(series.state), state: series.state, hour: observation.readHour, value });
  }
  return { observation, rows, selectionUnknown };
}

/** Strength is a fixed5 fraction; resistance/compliance are percentage units. */
export function strengthPercent(raw: string): string {
  return `${formatAmount((BigInt(raw) * 100n).toString(), 5, 5)}%`;
}

export function percent(raw: string): string {
  return `${formatAmount(raw, 5, 5)}%`;
}
/** Percentage cells round half away from zero, narrow to i32 and clamp to 0..100.
 * The result is formatted as a zero-decimal percent.
 * Only cell text follows this rule; tooltip, sort and bar inputs stay exact. */
export function percentCell(raw: string): string {
  const value = BigInt(raw);
  const rounded = BigInt.asIntN(32, (value + (value < 0n ? -50000n : 50000n)) / 100000n);
  const clamped = rounded < 0n ? 0n : rounded > 100n ? 100n : rounded;
  return `${formatNativeFixed5(clamped * 100000n, 0)}%`;
}
/** Strength times 100 wraps, rounds half away from zero,
 * then narrows to a signed i32 percentage for the bar. Tooltip values
 * retain the exact fraction; bar geometry follows the native integer percent. */
export function strengthBar(raw: string): bigint {
  const percent = BigInt.asIntN(64, BigInt(raw) * 100n);
  const biased = BigInt.asIntN(64, percent + (percent < 0n ? -50000n : 50000n));
  return BigInt.asIntN(32, biased / 100000n) * 1000n;
}
/** Garrison cell text formats the required amount with two fixed decimals;
 * only the bar displays strength. */
export function garrisonCell(value: OccupiedValues, cellBars: boolean): { text: string; bar: bigint | null; red: boolean } {
  if (value.garrisonRequiredRaw === null || BigInt(value.garrisonRequiredRaw) === 0n) return { text: '-', bar: null, red: false };
  if (BigInt(value.garrisonRequiredRaw) < 0n) return { text: '!', bar: cellBars ? 0n : null, red: true };
  return { text: formatNativeFixed5(value.garrisonRequiredRaw, 2),
    bar: cellBars && value.garrisonStrengthRaw !== null ? strengthBar(value.garrisonStrengthRaw) : null, red: value.garrisonRed === true };
}
export function occupationTooltip(row: OccupiedRow, countryName: (tag: string) => string): TooltipContent {
  const v = row.value;
  return { title: stateName(row), rows: [
    { label: 'Resistance', value: percent(v.resistanceRaw) }, { label: 'Resistance speed', value: `${formatSigned(v.resistanceSpeedRaw, 5, 5)} pp/day` },
    { label: 'Compliance', value: percent(v.complianceRaw) }, { label: 'Compliance speed', value: `${formatSigned(v.complianceSpeedRaw, 5, 5)} pp/day` },
    { label: 'Required manpower', value: v.requiredManpower === null ? '—' : formatFixed(BigInt(v.requiredManpower), 0, 0) },
    { label: 'Assigned manpower', value: v.assignedManpower === null ? '—' : formatFixed(BigInt(v.assignedManpower), 0, 0) },
    { label: 'Required garrison amount', value: v.garrisonRequiredRaw === null ? '—' : formatAmount(v.garrisonRequiredRaw, 5, 5) },
    { label: 'Garrison strength', value: v.garrisonStrengthRaw === null ? 'Not in this recording' : strengthPercent(v.garrisonStrengthRaw) },
    { label: 'Occupied country', value: v.occupied === null ? 'Unknown' : countryName(v.occupied) },
    ...(v.garrisonAlert ? [{ label: 'Garrison requirement not met.', tone: 'negative' as const }] : []),
  ] };
}
