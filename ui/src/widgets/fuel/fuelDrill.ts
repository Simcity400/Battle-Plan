import type { FuelAwardPart, FuelAwardsParams, FuelRead, FuelReply } from '../../generated/dashboard';
import { drillHour } from '../../charts/drill/drillState.ts';
import type { DrillAction, DrillState } from '../../charts/drill/drillState';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { object, signed32, unsigned32 } from '../../dashboard/validation.ts';
import { fuelAwardBasis } from './fuelData.ts';
import type { Span } from '../../charts/series';
import { runAt } from '../../charts/series.ts';

export interface FuelRecipientSelection {
  campaign: string; branch: string; consumer: number; basis: FuelAwardsParams; key: string; continuityStartHour: number;
}

export const fuelRecipientToken = (selection: FuelRecipientSelection) => setToken(JSON.stringify(selection));

/** Native country/recipient keys belong only to this campaign and branch. */
export function readFuelRecipient(token: unknown, campaign: string, branch: string): FuelRecipientSelection | null {
  if (typeof token !== 'string' || !token) return null;
  try {
    const value: unknown = JSON.parse(contentKey(token));
    if (!object(value) || value.campaign !== campaign || value.branch !== branch || !unsigned32(value.consumer) || value.consumer > 2
      || !signed32(value.continuityStartHour) || typeof value.key !== 'string' || !value.key || !object(value.basis) || value.basis.branchId !== branch
      || typeof value.basis.tag !== 'string' || !signed32(value.basis.hour) || typeof value.basis.country !== 'string'
      || !/^\d{1,10}$/.test(value.basis.country) || BigInt(value.basis.country) >= (1n << 32n) || !unsigned32(value.basis.countryInstance)) return null;
    return value as unknown as FuelRecipientSelection;
  } catch { return null; }
}

export function selectFuelRecipient(reply: FuelReply, read: FuelRead, part: FuelAwardPart, campaign: string): FuelRecipientSelection | null {
  const basis = fuelAwardBasis(reply, read);
  return basis && read.continuityStartHour !== null ? { campaign, branch: reply.branchId, consumer: part.kind, basis, key: part.key,
    continuityStartHour: read.continuityStartHour } : null;
}

/** Actor choices and restored pins cannot request or render another country's basis. */
export const fuelRecipientAtPick = (selection: FuelRecipientSelection | null | undefined, tag: string | null | undefined) =>
  selection?.basis.tag === tag ? selection ?? null : null;
export function fuelRecipientForReply(selection: FuelRecipientSelection | null | undefined, reply: Pick<FuelReply, 'branchId' | 'tag'>, campaign: string,
  aliases?: ReadonlyMap<string, readonly Span[]>) {
  return selection?.campaign === campaign && selection.branch === reply.branchId && selection.basis.branchId === reply.branchId
    && (selection.basis.tag === reply.tag && !aliases || !!runAt(aliases?.get(selection.basis.tag) ?? [], selection.basis.hour) && !!aliases?.get(reply.tag)?.length) ? selection : null;
}
export function fuelRecipientBasis(reply: Pick<FuelReply, 'branchId' | 'tag'>, read: FuelRead | null | undefined, selection: FuelRecipientSelection | null | undefined, campaign: string,
  aliases?: ReadonlyMap<string, readonly Span[]>) {
  return selection ? fuelRecipientForReply(selection, reply, campaign, aliases)?.basis ?? null : fuelAwardBasis(reply, read);
}

/** A changed country/refused read cannot donate rows to a held recipient. */
export function sameFuelActor(selection: FuelRecipientSelection, read: FuelRead | null | undefined) {
  return read?.country === selection.basis.country && read.countryInstance === selection.basis.countryInstance && read.continuityStartHour === selection.continuityStartHour;
}

export const fuelExtentToken = (campaign: string, branch: string, extent: readonly [number, number]) => setToken(JSON.stringify([campaign, branch, ...extent]));
export function readFuelExtent(token: unknown, campaign: string, branch: string): [number, number] | null {
  if (typeof token !== 'string' || !token) return null;
  try {
    const value: unknown = JSON.parse(contentKey(token));
    return Array.isArray(value) && value.length === 4 && value[0] === campaign && value[1] === branch
      && signed32(value[2]) && signed32(value[3]) && value[2] <= value[3] ? [value[2], value[3]] : null;
  } catch { return null; }
}

/** A held main view opens at the actual drawn hour; explicit Follow uses the real clock. */
export function fuelOpen(breakdown: string, hour: number, playing: boolean, follows: boolean, playhead: number | null): DrillAction {
  return { type: 'open', breakdown, hour, playing, playhead: follows ? playhead : null };
}
export const fuelMoment = (drill: DrillState, saved: boolean, shownHour: number, clockHour: number | null) =>
  saved && drill.breakdown === null ? Math.floor(shownHour) : drillHour(drill, clockHour);
