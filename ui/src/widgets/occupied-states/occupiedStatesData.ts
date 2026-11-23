import { formatAmount, formatFixed, formatSigned } from '../../charts/valueAxis.ts';
import { amountCell, strengthBar } from '../../draw/tables/logic.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { artworkUrlValue } from '../../dashboard/payload.ts';
import type { OccupiedState as RepliedState, OccupiedStatesReply as Replied } from '../../generated/dashboard';

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

export const occupiedStructure = (reply: OccupiedStatesReply) => JSON.stringify([reply.complete, reply.missing, reply.warnings,
  reply.states.some(state => state.occupier === null), reply.states.some(state => state.occupied === null)]);

export interface OccupiedDetail { value: OccupiedState; law: OccupiedStatesReply['laws'][number] | null; strengthMissing: string }
export function occupiedDetail(reply: OccupiedStatesReply, key: string): OccupiedDetail | null {
  const value = reply.states.find(state => String(state.state) === key);
  return value ? { value, law: reply.laws.find(law => law.token === value.law) ?? null, strengthMissing: reply.strengthMissing } : null;
}

export interface OccupiedRow { key: string; state: number; value: OccupiedState }
export type CountryRole = 'occupier' | 'occupied';
export const stateName = (row: OccupiedRow) => row.value.name ?? `State ${row.state}`;

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

export function emptyReason(reply: OccupiedStatesReply, view: { rows: readonly unknown[]; selectionUnknown: boolean; membership: string | null }): string | null {
  if (reply.missing !== null) return `Occupied states weren’t recorded at this time: ${reply.missing}`;
  if (view.rows.length) return null;
  if (view.membership) return view.membership;
  if (view.selectionUnknown) return 'Occupied states unavailable for these countries.';
  return reply.complete ? 'No occupied states match these countries at this moment.' : 'Some occupied states are missing.';
}

export function liveRaw(values: LiveValues<number>, row: string, column: string): string | null {
  const value = values.get(liveKey(row, column));
  return value === undefined || !Number.isFinite(value) ? null : BigInt(Math.trunc(value)).toString();
}

export const strengthPercent = (raw: string) => `${formatAmount((BigInt(raw) * 100n).toString(), 5, 5)}%`;
export const percent = (raw: string) => `${formatAmount(raw, 5, 5)}%`;

export { percentCell, strengthBar } from '../../draw/tables/logic.ts';

export const garrisonText = (raw: number) => amountCell(BigInt(Math.trunc(raw)));

export function garrisonCell(value: OccupiedState): { bar: 0n | 'cell' | null; red: boolean } {
  if (value.garrisonRequiredSign === null || value.garrisonRequiredSign === 0) return { bar: null, red: false };
  if (value.garrisonRequiredSign < 0) return { bar: 0n, red: true };
  return { bar: 'cell', red: false };
}

export const STRENGTH = 'garrisonStrength';

export const garrisonBar = (value: number) => strengthBar(BigInt(Math.trunc(value)).toString());

export function garrisonTone(threshold: string | null): (value: number) => string | undefined {
  const below = threshold === null ? null : BigInt(threshold);
  return value => below !== null && BigInt(Math.trunc(value)) < below ? 'occupation-garrison-alert' : undefined;
}

function strengthText(row: OccupiedRow, values: LiveValues<number>, strengthMissing: string): string {
  const at = liveRaw(values, row.key, STRENGTH);
  if (at !== null) return strengthPercent(at);
  if (row.value.garrisonMissing !== null) return row.value.garrisonMissing;
  const why = values.missing(liveKey(row.key, STRENGTH));
  return why === NOT_NAMED ? strengthMissing : why;
}

export const NOT_NAMED = 'Not a live cell of this reply';

export const OCCUPATION_TOOLTIP_COLUMNS = ['resistance', 'resistanceSpeed', 'compliance', 'complianceSpeed', 'requiredManpower', 'assignedManpower', 'garrison', STRENGTH] as const;
export const occupationTooltipKeys = (row: string) => OCCUPATION_TOOLTIP_COLUMNS.map(column => liveKey(row, column));

export function occupationTooltip(row: OccupiedRow, values: LiveValues<number>, countryName: (tag: string) => string, strengthMissing: string,
  lawName: (token: string) => string): TooltipContent {
  const v = row.value;
  const shown = (label: string, value: string) => ({ label, value });
  const missing = (label: string, why: string) => ({ label, value: why, tone: 'muted' as const });
  const cell = (label: string, column: string, show: (raw: string) => string) => {
    const at = liveRaw(values, row.key, column);
    return at === null ? missing(label, values.missing(liveKey(row.key, column))) : shown(label, show(at));
  };
  const whole = (raw: string) => formatFixed(BigInt(raw), 0, 0);
  const garrison = (label: string, column: string, show: (raw: string) => string) => v.garrisonRequiredSign === null ? shown(label, '—') : cell(label, column, show);
  const assigned = !v.assignedKnown ? shown('Assigned manpower', '—') : liveRaw(values, row.key, 'assignedManpower') === null
    && values.missing(liveKey(row.key, 'assignedManpower')) === NOT_NAMED ? shown('Assigned manpower', '0') : cell('Assigned manpower', 'assignedManpower', whole);
  const country = (label: string, tag: string | null, unknown: string) => tag === null ? missing(label, unknown) : shown(label, countryName(tag));
  const strength = liveRaw(values, row.key, STRENGTH) === null ? missing('Garrison strength', strengthText(row, values, strengthMissing))
    : shown('Garrison strength', strengthText(row, values, strengthMissing));
  return { title: stateName(row), rows: [
    country('Owner', v.owner, 'Unknown country'),
    country('Occupier', v.occupier, 'Unknown country'),
    cell('Resistance', 'resistance', percent),
    cell('Resistance speed', 'resistanceSpeed', raw => `${formatSigned(raw, 5, 5)} pp/day`),
    cell('Compliance', 'compliance', percent),
    cell('Compliance speed', 'complianceSpeed', raw => `${formatSigned(raw, 5, 5)} pp/day`),
    garrison('Required manpower', 'requiredManpower', whole),
    assigned,
    garrison('Required garrison amount', 'garrison', raw => formatAmount(raw, 5, 5)),
    strength,
    v.law === null ? missing('Occupation law', v.lawMissing ?? 'Not recorded') : shown('Occupation law', lawName(v.law)),
    country('Occupied country', v.occupied, 'Unknown'),
    ...(v.garrisonAlert ? [{ label: 'Garrison requirement not met.', tone: 'negative' as const }] : []),
  ] };
}
