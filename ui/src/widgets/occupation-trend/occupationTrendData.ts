import { formatAmount } from '../../charts/valueAxis.ts';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { OccupationDetailReply } from '../../generated/dashboard';

export type OccupationMeasure = 'resistance' | 'compliance';
export type ShownMeasure = OccupationMeasure | 'both';
export const SERIES = 'occupier-totals';
export const measureName = (measure: OccupationMeasure) => measure === 'resistance' ? 'Resistance' : 'Compliance';
export const otherMeasure = (measure: OccupationMeasure): OccupationMeasure => measure === 'resistance' ? 'compliance' : 'resistance';
export const rankingMeasure = (shown: ShownMeasure, ranking: OccupationMeasure): OccupationMeasure => shown === 'both' ? ranking : shown;
export const occupationPercent = (raw: string, digits = 5) => `${formatAmount(raw, 5, digits)}%`;

const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const text = (v: unknown) => v === null || typeof v === 'string';
const raw = (v: unknown) => v === null || (typeof v === 'string' && /^-?\d{1,19}$/.test(v));

export function validateOccupationDetail(data: unknown): OccupationDetailReply {
  if (!object(data) || !text(data.collaborationMissing) || !Array.isArray(data.warnings) || !data.warnings.every(item => typeof item === 'string')
    || !Array.isArray(data.occupiers) || !data.occupiers.every(row => object(row) && (typeof row.country === 'string' || typeof row.country === 'number') && text(row.missing) && Array.isArray(row.collaboration)
      && row.collaboration.every(item => object(item) && Number.isInteger(item.slot) && text(item.tag) && raw(item.progressRaw)))) {
    throw new Error('The occupation reply is malformed.');
  }
  return data as unknown as OccupationDetailReply;
}

export interface OccupierTotals { resistanceRaw: string; complianceRaw: string; states: number; heldStates: number | null; heldComplete: boolean }

export function occupationMean(sum: number, count: number): string | null {
  if (!Number.isFinite(sum) || !Number.isInteger(count) || count <= 0) return null;
  return (BigInt.asIntN(64, BigInt(Math.trunc(sum)) * 100000n) / (BigInt(count) * 100000n)).toString();
}

export function totalsOf(values: LiveValues<number>, tag: string): OccupierTotals | null {
  const cell = (column: string) => values.get(liveKey(tag, column));
  const states = cell('states'), resistance = cell('resistanceSum'), compliance = cell('complianceSum');
  if (states === undefined || resistance === undefined || compliance === undefined) return null;
  const resistanceRaw = occupationMean(resistance, states), complianceRaw = occupationMean(compliance, states);
  if (resistanceRaw === null || complianceRaw === null) return null;
  const held = cell('heldStates');
  return { resistanceRaw, complianceRaw, states, heldStates: held ?? null, heldComplete: cell('heldComplete') === 1 };
}

export function totalsMissing(values: LiveValues<number>, tag: string): string {
  return values.missing(liveKey(tag, 'states'));
}

export const heldCoverage = (totals: OccupierTotals) => totals.heldComplete && totals.heldStates !== null
  ? `${totals.states}/${totals.heldStates} states` : `${totals.states} states; total unknown`;
