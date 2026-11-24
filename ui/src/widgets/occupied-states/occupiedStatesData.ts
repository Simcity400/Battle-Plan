import { artworkUrlValue } from '../../dashboard/payload.ts';
import type { Value } from '../../families/value.ts';
import type { OccupiedState as RepliedState, OccupiedStatesReply as Replied } from '../../generated/dashboard';

export type OccupiedState = Omit<RepliedState, 'garrisonStrengthRaw' | 'garrisonRed'>;
export type OccupiedStatesReply = Omit<Replied, 'states'> & { states: OccupiedState[]; garrisonRedRaw: string | null };

const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const text = (v: unknown) => v === null || typeof v === 'string';
const tag = (v: unknown) => v === null || (typeof v === 'string' && v.length > 0 && !v.includes('\0'));
const raw = (v: unknown) => v === null || (typeof v === 'string' && /^-?\d{1,19}$/.test(v) && BigInt(v) >= -(1n << 63n) && BigInt(v) < (1n << 63n));
const sign = (v: unknown) => v === null || v === -1 || v === 0 || v === 1;

const STATE: [keyof OccupiedState, (v: unknown) => boolean][] = [
  ['name', text], ['nameMissing', text], ['owner', tag], ['occupier', tag], ['occupied', tag],
  ['garrisonRequiredSign', sign], ['garrisonAlert', v => typeof v === 'boolean'],
  ['assignedKnown', v => typeof v === 'boolean'], ['garrisonMissing', text], ['law', text], ['lawMissing', text],
];

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

export interface OccupiedRow { key: string; state: number; value: OccupiedState; occupiedName?: Value<string> }
export type CountryRole = 'occupier' | 'occupied';

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

export function garrisonCell(value: OccupiedState): { bar: 0n | 'cell' | null; red: boolean } {
  if (value.garrisonRequiredSign === null || value.garrisonRequiredSign === 0) return { bar: null, red: false };
  if (value.garrisonRequiredSign < 0) return { bar: 0n, red: true };
  return { bar: 'cell', red: false };
}

export const STRENGTH = 'garrisonStrength';
export const NOT_NAMED = 'Not a live cell of this reply';
