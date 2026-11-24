import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';
import { BANDS, REPAIR_NOTE, dockyardItems, dockyardMonth, dockyardTotal, repairAmount, repairsRead, validateDockyardSpendingReply } from './dockyardData';
import { PurposeSpendingWidget } from './PurposeSpending';
import type { PurposeSpendingKind } from './PurposeSpending';

export const DOCKYARD: PurposeSpendingKind = {
  name: 'Dockyard spending', measure: 'dockyard IC',
  root: { id: 'all', label: 'All dockyard IC' }, bands: BANDS, options: OPTIONS, repairs: true,
  validate: validateDockyardSpendingReply, read: repairsRead,
  items: (data, _options, window, colours) => dockyardItems(data, window, colours),
  total: (data, _options, window) => dockyardTotal(data, window),
  other: (data, window) => data.repairs ? repairAmount(data, data.repairs.total, window, window.hour) : { exact: null, reason: 'notRecorded' },
  month: (data, _options, first, last, branchFirst) => dockyardMonth(data, first, last, branchFirst),
  note: REPAIR_NOTE,
};

export default function DockyardSpendingWidget(props: WidgetProps) {
  return <PurposeSpendingWidget kind={DOCKYARD} {...props} />;
}
