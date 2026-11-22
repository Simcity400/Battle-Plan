import { contentKey, setToken } from '../../dashboard/options.ts';
import { object } from '../../dashboard/validation.ts';
import type { Span } from '../../charts/series';
import { runAt } from '../../charts/series.ts';
import type { FuelAwardPart, FuelReadAt } from '../../generated/dashboard';

export interface FuelBasis { branchId: string; tag: string; hour: number; country: number; countryInstance: number }

const int = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
export const isBasis = (value: unknown, branch: string): value is FuelBasis => object(value) && value.branchId === branch && typeof value.tag === 'string'
  && value.tag.length > 0 && int(value.hour) && int(value.country) && value.country >= 0 && int(value.countryInstance) && value.countryInstance >= 0;

export function readBasis(branchId: string, tag: string, read: Pick<FuelReadAt, 'readHour' | 'country' | 'countryInstance'> | null | undefined): FuelBasis | null {
  return read && read.readHour !== null && read.countryInstance !== null
    ? { branchId, tag, hour: read.readHour, country: read.country, countryInstance: read.countryInstance } : null;
}

export interface FuelRecipientSelection { campaign: string; branch: string; consumer: number; basis: FuelBasis; key: string; name: string | null }

export const fuelRecipientToken = (selection: FuelRecipientSelection) => setToken(JSON.stringify(selection));

export function readFuelRecipient(token: unknown, campaign: string, branch: string): FuelRecipientSelection | null {
  if (typeof token !== 'string' || !token) return null;
  try {
    const value: unknown = JSON.parse(contentKey(token));
    if (!object(value) || value.campaign !== campaign || value.branch !== branch || !int(value.consumer) || value.consumer < 0 || value.consumer > 2
      || typeof value.key !== 'string' || !value.key || !isBasis(value.basis, branch) || !(value.name === null || typeof value.name === 'string')) return null;
    return value as unknown as FuelRecipientSelection;
  } catch { return null; }
}

export function selectFuelRecipient(campaign: string, basis: FuelBasis, part: FuelAwardPart): FuelRecipientSelection {
  return { campaign, branch: basis.branchId, consumer: part.kind, basis, key: part.key, name: part.recordedName };
}

export function fuelRecipientFor<T extends { campaign: string; branch: string; basis: FuelBasis }>(selection: T | null | undefined, campaign: string, branch: string,
  tag: string, aliases?: ReadonlyMap<string, readonly Span[]>): T | null {
  return selection?.campaign === campaign && selection.branch === branch && selection.basis.branchId === branch
    && (selection.basis.tag === tag && !aliases || !!runAt(aliases?.get(selection.basis.tag) ?? [], selection.basis.hour) && !!aliases?.get(tag)?.length) ? selection : null;
}

export const sameFuelActor = (basis: FuelBasis, read: Pick<FuelReadAt, 'country' | 'countryInstance'> | null | undefined) =>
  read?.country === basis.country && read.countryInstance === basis.countryInstance;

export const fuelExtentToken = (campaign: string, branch: string, extent: readonly [number, number]) => setToken(JSON.stringify([campaign, branch, ...extent]));
export function readFuelExtent(token: unknown, campaign: string, branch: string): [number, number] | null {
  if (typeof token !== 'string' || !token) return null;
  try {
    const value: unknown = JSON.parse(contentKey(token));
    return Array.isArray(value) && value.length === 4 && value[0] === campaign && value[1] === branch
      && int(value[2]) && int(value[3]) && value[2] <= value[3] ? [value[2], value[3]] : null;
  } catch { return null; }
}

export function detailWindows(extent: readonly [number, number], visible: readonly [number, number], hour: number | null, detailHours: number): number[] {
  const from = Math.max(extent[0], visible[0]), to = Math.min(extent[1], visible[1]);
  const wanted = new Set<number>();
  for (let window = Math.floor(from / detailHours); window <= Math.floor(to / detailHours); window++) wanted.add(window);
  if (hour !== null && hour >= extent[0] && hour <= extent[1]) wanted.add(Math.floor(hour / detailHours));
  return [...wanted].sort((a, b) => a - b);
}

export function joinedRuns<T extends { firstHour: number; lastHour: number }>(windows: readonly (readonly T[])[]): T[] {
  const runs = new Map<number, T>();
  for (const list of windows) for (const run of list) {
    const previous = runs.get(run.firstHour);
    if (!previous || run.lastHour > previous.lastHour) runs.set(run.firstHour, run);
  }
  return [...runs.values()].sort((a, b) => a.firstHour - b.firstHour);
}
