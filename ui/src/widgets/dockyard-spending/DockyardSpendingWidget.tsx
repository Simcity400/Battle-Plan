import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';
import { BANDS, REPAIR_NOTE, dockyardItems, repairAmount, repairsRead, validateDockyardSpendingReply } from './dockyardData';
import { PurposeSpendingWidget } from './PurposeSpending';
import type { PurposeSpendingKind } from './PurposeSpending';

export const DOCKYARD: PurposeSpendingKind = {
  name: 'Dockyard spending', measure: 'dockyard IC',
  root: { id: 'all', label: 'All dockyard IC' }, bands: BANDS, options: OPTIONS, repairs: true,
  validate: validateDockyardSpendingReply, read: repairsRead,
  items: (data, _options, window, colours) => dockyardItems(data, window, colours),
  other: (data, window) => data.repairs ? repairAmount(data, data.repairs.total, window, window.hour) : { exact: null, reason: 'notRecorded' },
  note: REPAIR_NOTE,
};

export default function DockyardSpendingWidget(props: WidgetProps) {
  return <PurposeSpendingWidget kind={DOCKYARD} {...props} />;
}
