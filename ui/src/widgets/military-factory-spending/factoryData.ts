import type { SunburstItem } from '../../charts/sunburst/layout';
import type { SpendingData, SpendingWindow } from '../production-spending/spendingData';
import { REMAINDER_BANDS, colouredSplit, domainLeaves, purposeItems } from '../dockyard-spending/dockyardData.ts';
import type { PurposeSplit } from '../dockyard-spending/dockyardData';
import type { Domain } from '../../generated/dashboard';

const PURPOSES = [
  { id: 'new', label: 'New' },
  { id: 'converted', label: 'Converted' },
] as const;

const PURPOSE_NOTE = 'Whether some of it was converted was not recorded.';
const SPLIT: PurposeSplit = { purposes: [{ key: PURPOSES[0], leaf: 'new' }, { key: PURPOSES[1], leaf: 'converted' }], note: PURPOSE_NOTE };

export function factoryItems(reply: SpendingData, domain: Domain, window: SpendingWindow, colours: readonly string[] = []): SunburstItem[] {
  return purposeItems(reply, domainLeaves(reply, domain), window, colouredSplit(SPLIT, colours));
}

export const BANDS = [...PURPOSES, ...REMAINDER_BANDS];
