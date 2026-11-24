import type { SunburstItem, SunburstTotal } from '../../charts/sunburst/layout';
import { equipmentPeriodNote } from '../production-spending/spendingData.ts';
import type { SpendingWindow } from '../production-spending/spendingData';
import { REMAINDER_BANDS, colouredSplit, domainEquipmentTotal, domainLeaves, monthNote, monthTotal, purposeItems, reasonLabel, spentMonth } from '../dockyard-spending/dockyardData.ts';
import type { Exact, PurposeMonth, PurposeSplit, SpentMonth } from '../dockyard-spending/dockyardData';
import type { Domain } from '../../generated/dashboard';
import type { SpendingData } from '../production-spending/spendingData';

export const PURPOSES = [
  { id: 'new', label: 'New' },
  { id: 'converted', label: 'Converted' },
] as const;

const PURPOSE_NOTE = 'Whether some of it was converted was not recorded.';
const SPLIT: PurposeSplit = { purposes: [{ key: PURPOSES[0], leaf: 'new' }, { key: PURPOSES[1], leaf: 'converted' }], note: PURPOSE_NOTE };

export function factoryItems(reply: SpendingData, domain: Domain, window: SpendingWindow, colours: readonly string[] = []): SunburstItem[] {
  return purposeItems(reply, domainLeaves(reply, domain), window, colouredSplit(SPLIT, colours));
}

export function factoryTotal(reply: SpendingData, domain: Domain, window: SpendingWindow): SunburstTotal {
  const now = domainEquipmentTotal(reply, domain, window);
  const end = domainEquipmentTotal(reply, domain, { ...window, hour: window.end });
  const periodNote = equipmentPeriodNote(reply, window);
  const note = [now.reason ? reasonLabel(now.reason) : undefined, periodNote].filter(Boolean).join(' ') || undefined;
  if (now.exact != null) return {
    value: Number(now.exact), exact: now.exact, unknownValue: false, note,
    final: end.exact != null ? Number(end.exact) : end.recorded != null ? Number(end.recorded) : 0,
    finalUnknown: end.exact == null && end.recorded == null,
  };
  if (now.recorded != null) return {
    value: Number(now.recorded), exact: now.recorded, unknownValue: false,
    unknown: { [now.reason ?? 'notRecorded']: 1 }, note,
    final: end.exact != null ? Number(end.exact) : end.recorded != null ? Number(end.recorded) : 0,
    finalUnknown: end.exact == null && end.recorded == null,
  };
  return { value: null, exact: null, unknownValue: true, note, final: 0, finalUnknown: true };
}

export const BANDS = [...PURPOSES, ...REMAINDER_BANDS];

const part = (month: SpentMonth, purpose: 'new' | 'converted' | 'unknown' | 'refit'): Exact =>
  month.amounts ? { exact: month.amounts[purpose], reason: null } : { exact: null, reason: month.reason };

export function factoryMonth(reply: SpendingData, domain: Domain, first: number, last: number, branchFirst: number): PurposeMonth {
  const spent = spentMonth(reply, domainLeaves(reply, domain), first, last, branchFirst);
  const bands = [part(spent, 'new'), part(spent, 'converted'), part(spent, 'unknown'), part(spent, 'refit')];
  const purposes = spent.purposeUnproven || spent.unresolved;
  return { firstHour: first, lastHour: last, bands, total: monthTotal(spent, bands), note: monthNote(spent, PURPOSE_NOTE),
    uncertain: [purposes, purposes, spent.purposeUnproven, false] };
}
