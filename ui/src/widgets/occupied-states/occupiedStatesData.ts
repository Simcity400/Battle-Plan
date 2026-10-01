import { runAt } from '../../charts/series.ts';
import { formatAmount, formatFixed, formatNativeFixed5, formatSigned } from '../../charts/valueAxis.ts';
import { readNote } from '../../dashboard/reads.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { OccupiedObservation, OccupiedStatesReply, OccupiedValues } from '../../generated/dashboard';

const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const hour = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= -2147483648 && v <= 2147483647;
const nullable = (v: unknown) => v === null || typeof v === 'string';
const raw = (v: unknown) => typeof v === 'string' && /^-?\d{1,19}$/.test(v) && BigInt(v) >= -(1n << 63n) && BigInt(v) < (1n << 63n);
const tag = (v: unknown) => v === null || (typeof v === 'string' && v.length > 0 && !v.includes('\0'));
const span = (v: unknown): v is { firstHour: number; lastHour: number } => object(v) && hour(v.firstHour) && hour(v.lastHour) && v.firstHour <= v.lastHour;
const ordered = (v: unknown): v is (Record<string, unknown> & { firstHour: number; lastHour: number })[] => Array.isArray(v) && v.every((item, i) => span(item) && (!i || v[i - 1].lastHour < item.firstHour));
const observations = (v: unknown) => ordered(v) && v.every((item, index) => {
  const previous = v[index - 1];
  return hour(item.readHour) && item.readHour <= item.firstHour && typeof item.complete === 'boolean'
    && (!previous || Number(previous.readHour) <= item.readHour)
    && (previous?.lastHour === item.firstHour - 1 || item.readHour === item.firstHour);
});
const value = (v: unknown) => object(v) && nullable(v.name) && nullable(v.nameMissing) && tag(v.owner) && tag(v.occupier) && tag(v.occupied)
  && ['resistanceRaw', 'complianceRaw', 'resistanceSpeedRaw', 'complianceSpeedRaw'].every(key => raw(v[key]))
  && (v.garrisonStrengthRaw === null || raw(v.garrisonStrengthRaw))
  && (v.requiredManpower === null || hour(v.requiredManpower)) && (v.garrisonRequiredRaw === null || raw(v.garrisonRequiredRaw))
  && typeof v.garrisonAlert === 'boolean' && (v.garrisonRed === null || typeof v.garrisonRed === 'boolean')
  && (v.assignedManpower === null || hour(v.assignedManpower))
  && nullable(v.garrisonMissing) && nullable(v.law) && nullable(v.lawMissing);

export function validateOccupiedStatesReply(data: unknown): OccupiedStatesReply {
  const states = object(data) && Array.isArray(data.states) ? data.states : [];
  if (!object(data) || typeof data.branchId !== 'string' || !/^\d+$/.test(data.branchId) || data.decimals !== 5
    || !observations(data.observed)
    || !Array.isArray(data.states) || !data.states.every((series, i) => object(series) && Number.isInteger(series.state) && Number(series.state) >= 0 && Number(series.state) <= 4294967295
      && (!i || (object(states[i - 1]) && Number(states[i - 1].state) < Number(series.state))) && ordered(series.runs) && series.runs.every(run => object(run) && value(run.value)))
    || !Array.isArray(data.laws) || !data.laws.every(law => object(law) && typeof law.token === 'string' && nullable(law.name) && nullable(law.missing)
      && (law.icon === null || (typeof law.icon === 'string' && law.icon.startsWith('data:image/png;base64,'))))
    || new Set(data.laws.map(law => law.token)).size !== data.laws.length
    || typeof data.strengthMissing !== 'string' || !Array.isArray(data.warnings) || !data.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('The occupied states reply is malformed.');
  }
  const reply = data as unknown as OccupiedStatesReply;
  // A value run may span consecutive read intervals, but cannot bridge an
  // unobserved hour. Two ordered cursors avoid scanning all observations
  // once for every run in a large branch.
  for (const series of reply.states) {
    let index = 0;
    for (const run of series.runs) {
      while (index < reply.observed.length && reply.observed[index].lastHour < run.firstHour) index++;
      let end = run.firstHour - 1;
      for (let at = index; at < reply.observed.length && end < run.lastHour; at++) {
        const span = reply.observed[at];
        if (span.firstHour > end + 1) break;
        end = span.lastHour;
      }
      if (end < run.lastHour) throw new Error('The occupied states reply is malformed: a state run crosses unobserved hours.');
    }
  }
  return reply;
}

export interface OccupiedRow { key: string; state: number; hour: number; value: OccupiedValues }
export function occupiedAt(reply: OccupiedStatesReply, hour: number, countries: ReadonlySet<string> | null, role: 'occupier' | 'occupied'): { observation: OccupiedObservation | null; rows: OccupiedRow[] } {
  const observation = runAt(reply.observed, hour);
  if (!observation) return { observation: null, rows: [] };
  const rows: OccupiedRow[] = [];
  for (const series of reply.states) {
    const run = runAt(series.runs, hour);
    if (!run || (countries && (run.value[role] == null || !countries.has(run.value[role]!)))) continue;
    rows.push({ key: String(series.state), state: series.state, hour: observation.readHour, value: run.value });
  }
  return { observation, rows };
}

export function percent(raw: string): string {
  return `${formatAmount(raw, 5, 5)}%`;
}
/** P: 14155CC50 -> 14155BE80 rounds with 1424D8760, narrows to i32 and
 * clamps to 0..100. 141554BD0 -> 142259590 writes a zero-decimal percent.
 * Only cell text follows this rule; tooltip, sort and bar inputs stay exact. */
export function percentCell(raw: string): string {
  const value = BigInt(raw);
  const rounded = BigInt.asIntN(32, (value + (value < 0n ? -50000n : 50000n)) / 100000n);
  const clamped = rounded < 0n ? 0n : rounded > 100n ? 100n : rounded;
  return `${formatNativeFixed5(clamped * 100000n, 0)}%`;
}
/** P: 14155CC50 formats +0xF8 with Fixed5ToString@1424B6200(...,2);
 * only the bar displays strength. */
export function garrisonCell(value: OccupiedValues, cellBars: boolean): { text: string; bar: bigint | null; red: boolean } {
  if (value.garrisonRequiredRaw === null || BigInt(value.garrisonRequiredRaw) === 0n) return { text: '-', bar: null, red: false };
  if (value.garrisonAlert) return { text: '!', bar: cellBars ? 0n : null, red: true };
  return { text: formatNativeFixed5(value.garrisonRequiredRaw, 2),
    bar: cellBars && value.garrisonStrengthRaw !== null ? BigInt(value.garrisonStrengthRaw) : null, red: value.garrisonRed === true };
}
export function occupationTooltip(reply: OccupiedStatesReply, row: OccupiedRow, shownHour: number, countryName: (tag: string) => string): TooltipContent {
  const v = row.value;
  return { title: v.name || `State ${row.state}`, rows: [
    { label: 'Resistance', value: percent(v.resistanceRaw) }, { label: 'Resistance speed', value: `${formatSigned(v.resistanceSpeedRaw, 5, 5)} pp/day` },
    { label: 'Compliance', value: percent(v.complianceRaw) }, { label: 'Compliance speed', value: `${formatSigned(v.complianceSpeedRaw, 5, 5)} pp/day` },
    { label: 'Required manpower', value: v.requiredManpower === null ? '—' : formatFixed(BigInt(v.requiredManpower), 0, 0) },
    { label: 'Assigned manpower', value: v.assignedManpower === null ? '—' : formatFixed(BigInt(v.assignedManpower), 0, 0) },
    { label: 'Required garrison amount', value: v.garrisonRequiredRaw === null ? '—' : formatAmount(v.garrisonRequiredRaw, 5, 5) },
    { label: 'Garrison strength', value: v.garrisonStrengthRaw === null ? 'Not in this recording' : percent(v.garrisonStrengthRaw) },
    { label: 'Occupied country', value: v.occupied === null ? 'Unknown' : countryName(v.occupied) },
    ...(v.garrisonAlert ? [{ label: 'The recorded requirement failure flag is set; the game shows a red ! and zero strength.', tone: 'negative' as const }] : []),
    ...(v.nameMissing ? [{ label: v.nameMissing, tone: 'muted' as const }] : []),
    ...(v.garrisonMissing ? [{ label: v.garrisonMissing, tone: 'muted' as const }] : []),
    ...(v.lawMissing ? [{ label: v.lawMissing, tone: 'muted' as const }] : []),
  ], description: v.garrisonStrengthRaw === null ? reply.strengthMissing : undefined, note: readNote(row.hour, shownHour, 'occupation values are read once a day') };
}
