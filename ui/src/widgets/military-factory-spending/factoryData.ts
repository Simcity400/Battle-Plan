import type { BreakdownLeafItem as SunburstItem } from '../../families/breakdown/tree';
import type { SpendingWindow } from '../production-spending/spendingData';
import { REMAINDER_BANDS, colouredSplit, domainLeaves, monthNote, monthTotal, purposeItems, spentMonth } from '../dockyard-spending/dockyardData.ts';
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
