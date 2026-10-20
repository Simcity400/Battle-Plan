import { U32_MAX, U8_MAX, unsignedId } from './batch5Params.ts';
import type {
  AirMissionsParams, AirStatisticsParams, DoctrineMasteryParams, IntelCoverageParams, MinefieldsParams, NavalSupremacyParams,
  NavalYardsParams, OccupationReserveParams, RegionalWeatherParams,
} from './batch5Params';

/** The Batch 5 widgets' request parameters, built to their DTOs' exact field types. A builder
 * whose id is not the unsigned integer the backend reads returns null, and the widget asks for
 * nothing. */

export const airMissionsHistory = (branchId: string, tag: string): AirMissionsParams => ({ branchId, tag, view: 'history' });

export function airMissionsWings(branchId: string, tag: string, region: string, hour: number): AirMissionsParams | null {
  const id = unsignedId(region, U32_MAX);
  return id === null ? null : { branchId, tag, view: 'wings', region: id, hour };
}

export function airStatistics(branchId: string, tag: string, channel: string): AirStatisticsParams | null {
  const id = unsignedId(channel, U8_MAX);
  return id === null ? null : { branchId, tag, channel: id };
}

/** One day of either air product, for its theatre choices at that day. */
export const oneDay = <P extends AirMissionsParams | AirStatisticsParams>(params: P, day: number): P => ({ ...params, firstHour: day, lastHour: day });

export const minefieldsHistory = (branchId: string, tags?: readonly string[]): MinefieldsParams =>
  ({ branchId, view: 'history', ...tags ? { tags: [...tags] } : {} });

export function minefieldReports(branchId: string, region: string, hour: number): MinefieldsParams | null {
  const id = unsignedId(region, U32_MAX);
  return id === null ? null : { branchId, view: 'reports', region: id, hour };
}

export const navalSupremacy = (branchId: string, tag: string, firstHour: number, lastHour: number): NavalSupremacyParams =>
  ({ branchId, tag, firstHour, lastHour, daily: true });

export const weatherHistory = (branchId: string, firstHour: number, lastHour: number, show: 'conditions' | 'modifier' | 'temperature',
  modifier: string | null): RegionalWeatherParams => ({ branchId, firstHour, lastHour, daily: true, show, ...show === 'modifier' && modifier ? { modifier } : {} });

export const weatherProvinces = (branchId: string, region: string, hour: number): RegionalWeatherParams =>
  ({ branchId, view: 'provinces', region, hour });

export const doctrineMastery = (branchId: string, tag: string): DoctrineMasteryParams => ({ branchId, tag });

/** One hour's doctrine reply, for descriptions with the country's values at that read. */
export const doctrineMasteryAt = (branchId: string, tag: string, hour: number): DoctrineMasteryParams =>
  ({ branchId, tag, hour, firstHour: hour, lastHour: hour });

export const occupationRace = (branchId: string, tags?: readonly string[]): OccupationReserveParams =>
  ({ branchId, view: 'race', ...tags ? { tags: [...tags] } : {} });

export const occupationHolder = (branchId: string, tag: string): OccupationReserveParams => ({ branchId, view: 'holder', tag });

export const intelMatrix = (branchId: string, shown: number, tags?: readonly string[]): IntelCoverageParams =>
  ({ branchId, view: 'matrix', shown: Math.max(2, Math.floor(shown)), ...tags ? { tags: [...tags] } : {} });

export const intelPair = (branchId: string, owner: string, target: string): IntelCoverageParams => ({ branchId, view: 'pair', owner, target });

export const navalYards = (branchId: string, tag: string): NavalYardsParams => ({ branchId, tag });
