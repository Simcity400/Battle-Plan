import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveWindow } from '../../dashboard/model.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import type { TransferOccurrence } from './territoryData';

export function transferTime(widget: WidgetSpec, track: BoundTrack, tracks: BoundTrack[], position: number, fromOffset: number | null = null) {
  const window = resolveWindow(widget, track, tracks, position);
  const from = Math.max(widget.time.mode === 'range' ? Math.ceil(window?.firstHour ?? Infinity) : -Infinity,
    fromOffset == null ? -Infinity : track.branch.firstHour + fromOffset);
  return { moment: Math.floor(window?.lastHour ?? -Infinity), from };
}

export interface TransferSelection { ids: string[]; fromOffset: number | null; country: string | null }
export const transferSelectionToken = (campaign: string, branch: string, ids: readonly string[], fromOffset: number | null = null, country: string | null = null) =>
  setToken(campaignToken(campaign, JSON.stringify(country == null ? [branch, ids, fromOffset] : [branch, ids, fromOffset, country])));
export function transferSelection(token: string, campaign: string, branch: string): TransferSelection | null {
  if (!token) return { ids: [], fromOffset: null, country: null };
  const decoded = contentKey(token), prefix = `${campaign}:`;
  if (!decoded.startsWith(prefix)) return null;
  try {
    const value = JSON.parse(decoded.slice(prefix.length));
    const valid = Array.isArray(value) && (value.length === 3 || value.length === 4 && typeof value[3] === 'string') && value[0] === branch
      && Array.isArray(value[1]) && value[1].every(id => typeof id === 'string') && (value[2] === null || Number.isSafeInteger(value[2]));
    return valid ? { ids: [...new Set<string>(value[1])], fromOffset: value[2], country: value[3] ?? null } : null;
  } catch { return null; }
}

const LINK = /^\d+:source:(\d+):(\d+)target:(\d+):(\d+)$/;
export function linkEnds(id: string): [string, string] | null {
  const match = LINK.exec(id);
  if (!match) return null;
  const [slot, countryId, toSlot, toCountryId] = match.slice(1).map(Number);
  return [countryIdentityKey({ slot, countryId }), countryIdentityKey({ slot: toSlot, countryId: toCountryId })];
}

export function selectedTransfers(list: readonly TransferOccurrence[], selection: TransferSelection): { shown: readonly TransferOccurrence[]; missing: number } {
  if (selection.country != null) return { shown: list.filter(occurrence => occurrence.to.key === selection.country || occurrence.from?.key === selection.country), missing: 0 };
  if (!selection.ids.length) return { shown: list, missing: 0 };
  const pairs = new Set<string>();
  let missing = 0;
  for (const id of selection.ids) {
    const ends = linkEnds(id);
    if (ends) pairs.add(JSON.stringify(ends)); else missing++;
  }
  const shown = list.filter(occurrence => occurrence.from != null && pairs.has(JSON.stringify([occurrence.from.key, occurrence.to.key])));
  const present = new Set(shown.map(occurrence => JSON.stringify([occurrence.from!.key, occurrence.to.key])));
  for (const pair of pairs) if (!present.has(pair)) missing++;
  return { shown, missing };
}
