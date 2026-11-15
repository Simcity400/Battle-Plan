import { formatAmount, formatFixed, formatNativeFixed5, formatSigned } from '../../charts/valueAxis.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { artworkUrlValue } from '../../dashboard/payload.ts';
import type { OccupiedState as RepliedState, OccupiedStatesReply as Replied } from '../../generated/dashboard';

// DT1 moved the strength into a live cell and the red threshold into the reply: the contract regenerated after it
// carries these two types as they are here; then import them from it directly.
export type OccupiedState = Omit<RepliedState, 'garrisonStrengthRaw' | 'garrisonRed'>;
export type OccupiedStatesReply = Omit<Replied, 'states'> & { states: OccupiedState[]; garrisonRedRaw: string | null };

const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const text = (v: unknown) => v === null || typeof v === 'string';
const tag = (v: unknown) => v === null || (typeof v === 'string' && v.length > 0 && !v.includes('\0'));
const raw = (v: unknown) => v === null || (typeof v === 'string' && /^-?\d{1,19}$/.test(v) && BigInt(v) >= -(1n << 63n) && BigInt(v) < (1n << 63n));
const sign = (v: unknown) => v === null || v === -1 || v === 0 || v === 1;

const STATE: [keyof OccupiedState, (v: unknown) => boolean][] = [['name', text], ['nameMissing', text], ['owner', tag], ['occupier', tag], ['occupied', tag],
  ['garrisonRequiredSign', sign], ['garrisonAlert', v => typeof v === 'boolean'],
  ['assignedKnown', v => typeof v === 'boolean'], ['garrisonMissing', text], ['law', text], ['lawMissing', text]];

export function validateOccupiedStates(data: unknown): OccupiedStatesReply {
  let previous = -1;
  if (!object(data) || !Array.isArray(data.states) || typeof data.complete !== 'boolean' || !text(data.missing) || typeof data.strengthMissing !== 'string' || !raw(data.garrisonRedRaw)
    || !Array.isArray(data.warnings) || !data.warnings.every(warning => typeof warning === 'string')
    || !Array.isArray(data.laws) || !data.laws.every(law => object(law) && typeof law.token === 'string' && text(law.name) && text(law.missing) && (law.icon === null || artworkUrlValue(law.icon)))
    || !data.states.every(state => {
      const ordered = object(state) && Number.isInteger(state.state) && Number(state.state) > previous && Number(state.state) <= 4294967295;
      if (ordered) previous = Number(state.state);
      return ordered && !('garrisonStrengthRaw' in state) && STATE.every(([key, valid]) => valid(state[key]));
    })) throw new Error('The occupied states reply is malformed.');
  return data as unknown as OccupiedStatesReply;
}

export interface OccupiedRow { key: string; state: number; value: OccupiedState }
export type CountryRole = 'occupier' | 'occupied';
export const stateName = (row: OccupiedRow) => row.value.name ?? `State ${row.state}`;

/** The reply's states whose occupier (or occupied country) `matches` admits; a state whose country is not named cannot
 * be matched, and says so, as one left out while the choice's membership is not recorded (`unrecorded`, its reason). */
export function occupiedRows(reply: OccupiedStatesReply, matches: ((tag: string) => boolean) | null, role: CountryRole, unrecorded: string | null = null):
  { rows: OccupiedRow[]; selectionUnknown: boolean; membership: string | null } {
  const rows: OccupiedRow[] = [];
  let selectionUnknown = false, membership: string | null = null;
  for (const value of reply.states) {
    if (matches) {
      const country = value[role];
      if (country === null) { selectionUnknown = true; continue; }
      if (!matches(country)) { membership ??= unrecorded; continue; }
    }
    rows.push({ key: String(value.state), state: value.state, value });
  }
  return { rows, selectionUnknown, membership };
}

/** A live cell's raw fixed5 value as an exact decimal string; null while it has none. */
export function liveRaw(values: LiveValues<number>, row: string, column: string): string | null {
  const value = values.get(liveKey(row, column));
  return value === undefined || !Number.isFinite(value) ? null : BigInt(Math.trunc(value)).toString();
}

/** Strength is a fixed5 fraction; resistance and compliance are percentage units. */
export const strengthPercent = (raw: string) => `${formatAmount((BigInt(raw) * 100n).toString(), 5, 5)}%`;
export const percent = (raw: string) => `${formatAmount(raw, 5, 5)}%`;

/** Percentage cells round half away from zero, narrow to i32 and clamp to 0..100, shown without decimals. Only cell
 * text follows this rule; the tooltip, the sort and the bar stay exact. */
export function percentCell(raw: string): string {
  const value = BigInt(raw);
  const rounded = BigInt.asIntN(32, (value + (value < 0n ? -50000n : 50000n)) / 100000n);
  const clamped = rounded < 0n ? 0n : rounded > 100n ? 100n : rounded;
  return `${formatNativeFixed5(clamped * 100000n, 0)}%`;
}

/** Strength times 100 wraps, rounds half away from zero, then narrows to a signed i32 percentage for the bar. */
export function strengthBar(raw: string): bigint {
  const percent = BigInt.asIntN(64, BigInt(raw) * 100n);
  const biased = BigInt.asIntN(64, percent + (percent < 0n ? -50000n : 50000n));
  return BigInt.asIntN(32, biased / 100000n) * 1000n;
}

/** The garrison cell's text, painted from its required amount's live cell: two decimals; none required "-", below zero
 * "!" (unmet). */
export function garrisonText(raw: number): string {
  const amount = BigInt(Math.trunc(raw));
  return amount === 0n ? '-' : amount < 0n ? '!' : formatNativeFixed5(amount, 2);
}

/** The garrison cell's structure: its bar (`'cell'`: the strength's live cell, `garrisonBar`; empty where unmet; none
 * where nothing is required or no garrison row) and whether it shows red whatever the strength (unmet). The amount it
 * shows is a live cell (`garrisonText`). */
export function garrisonCell(value: OccupiedState): { bar: 0n | 'cell' | null; red: boolean } {
  if (value.garrisonRequiredSign === null || value.garrisonRequiredSign === 0) return { bar: null, red: false };
  if (value.garrisonRequiredSign < 0) return { bar: 0n, red: true };
  return { bar: 'cell', red: false };
}

/** The live strength cell's id. */
export const STRENGTH = 'garrisonStrength';

/** The garrison bar's length painted from the strength's live cell: `strengthBar`'s narrowing. */
export const garrisonBar = (value: number) => strengthBar(BigInt(Math.trunc(value)).toString());

/** The red the strength's live cell paints: below the content's threshold; never where the content does not say. */
export function garrisonTone(threshold: string | null): (value: number) => string | undefined {
  const below = threshold === null ? null : BigInt(threshold);
  return value => below !== null && BigInt(Math.trunc(value)) < below ? 'occupation-garrison-alert' : undefined;
}

/** The strength's text at the hour shown: its cell, else why the reply named none, else why the cell has no value. */
function strengthText(row: OccupiedRow, values: LiveValues<number>, strengthMissing: string): string {
  const at = liveRaw(values, row.key, STRENGTH);
  if (at !== null) return strengthPercent(at);
  if (row.value.garrisonMissing !== null) return row.value.garrisonMissing;
  const why = values.missing(liveKey(row.key, STRENGTH));
  return why === NOT_NAMED ? strengthMissing : why;
}

/** What the live values say of a cell the reply did not name (`sectionValues`). */
export const NOT_NAMED = 'Not a live cell of this reply';

/** A row's tooltip: its live values at the hour shown (each missing one with why), then its garrison. */
export function occupationTooltip(row: OccupiedRow, values: LiveValues<number>, countryName: (tag: string) => string, strengthMissing: string): TooltipContent {
  const v = row.value;
  const cell = (column: string, show: (raw: string) => string) => {
    const at = liveRaw(values, row.key, column);
    return at === null ? values.missing(liveKey(row.key, column)) : show(at);
  };
  const whole = (raw: string) => formatFixed(BigInt(raw), 0, 0);
  // A garrison's numbers are cells only with a garrison row; assigned manpower has no cell where no row is assigned (zero).
  const garrison = (column: string, show: (raw: string) => string) => v.garrisonRequiredSign === null ? '—' : cell(column, show);
  const assigned = !v.assignedKnown ? '—' : liveRaw(values, row.key, 'assignedManpower') === null
    && values.missing(liveKey(row.key, 'assignedManpower')) === NOT_NAMED ? '0' : cell('assignedManpower', whole);
  return { title: stateName(row), rows: [
    { label: 'Resistance', value: cell('resistance', percent) },
    { label: 'Resistance speed', value: cell('resistanceSpeed', raw => `${formatSigned(raw, 5, 5)} pp/day`) },
    { label: 'Compliance', value: cell('compliance', percent) },
    { label: 'Compliance speed', value: cell('complianceSpeed', raw => `${formatSigned(raw, 5, 5)} pp/day`) },
    { label: 'Required manpower', value: garrison('requiredManpower', whole) },
    { label: 'Assigned manpower', value: assigned },
    { label: 'Required garrison amount', value: garrison('garrison', raw => formatAmount(raw, 5, 5)) },
    { label: 'Garrison strength', value: strengthText(row, values, strengthMissing) },
    { label: 'Occupied country', value: v.occupied === null ? 'Unknown' : countryName(v.occupied) },
    ...(v.garrisonAlert ? [{ label: 'Garrison requirement not met.', tone: 'negative' as const }] : []),
  ] };
}
