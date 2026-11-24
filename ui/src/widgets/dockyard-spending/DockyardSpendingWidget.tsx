import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';
import { BANDS, REPAIR_NOTE, dockyardMonth, repairsRead, validateDockyardSpendingReply } from './dockyardData';
import { PurposeSpendingWidget, type PurposeSpendingKind } from './PurposeSpending';

export const DOCKYARD: PurposeSpendingKind = {
  name: 'Dockyard spending',
  measure: 'dockyard IC',
  mode: 'dockyard',
  domain: () => 'naval',
  bands: BANDS,
  options: OPTIONS,
  repairs: true,
  validate: validateDockyardSpendingReply,
  read: repairsRead,
  month: (data, _options, first, last, branchFirst) => dockyardMonth(data, first, last, branchFirst),
  note: REPAIR_NOTE,
};

export default function DockyardSpendingWidget(props: WidgetProps) {
  return <PurposeSpendingWidget kind={DOCKYARD} {...props} />;
}
