import type { WidgetProps } from '../../dashboard/definitions';
import type { DockyardSpendingReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { BANDS, REPAIR_NOTE, dockyardItems, exactOf, monthColumns, monthNote, monthTotal, repairsRead, validateDockyardSpendingReply } from './dockyardData';
import { PurposeSpendingWidget } from './PurposeSpending';
import type { PurposeSpendingKind } from './PurposeSpending';

export const DOCKYARD: PurposeSpendingKind<DockyardSpendingReply> = {
  product: 'dockyard-spending', name: 'Dockyard spending', measure: 'dockyard IC',
  root: { id: 'all', label: 'All dockyard IC' }, bands: BANDS, options: OPTIONS,
  validate: validateDockyardSpendingReply, read: repairsRead,
  items: dockyardItems, other: data => exactOf(data.repairs.hour),
  columns: (data, decimals) => monthColumns(data.months, decimals, data.repairs.denominator),
  month: (data, index) => {
    const month = data.months[index];
    const denominator = BigInt(data.repairs.denominator);
    return { firstHour: month.firstHour, lastHour: month.lastHour, note: monthNote(month),
      bands: [exactOf(month.newShips), exactOf(month.refits), { ...exactOf(month.repairs), denominator }, exactOf(month.unrecorded), exactOf(month.other)],
      total: monthTotal(month, data.repairs.denominator) };
  },
  monthRow: (data, index) => {
    const month = data.months[index];
    return { path: [String(month.firstHour)], newShips: month.newShips.totalRaw, refits: month.refits.totalRaw,
      repairs: month.repairs.totalRaw, unrecorded: month.unrecorded.totalRaw, other: month.other.totalRaw, purposeUnproven: month.purposeUnproven, unresolved: month.unresolved };
  },
  note: REPAIR_NOTE,
};

export default function DockyardSpendingWidget(props: WidgetProps) {
  return <PurposeSpendingWidget kind={DOCKYARD} {...props} />;
}
