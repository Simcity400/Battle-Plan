import type { WidgetProps } from '../../dashboard/definitions';
import type { MilitaryFactorySpendingReply } from '../../generated/dashboard';
import { exactOf } from '../dockyard-spending/dockyardData';
import { PurposeSpendingWidget } from '../dockyard-spending/PurposeSpending';
import type { PurposeSpendingKind } from '../dockyard-spending/PurposeSpending';
import { OPTIONS } from './definition';
import { BANDS, factoryColumns, factoryItems, factoryMonthNote, factoryMonthTotal, validateFactorySpendingReply } from './factoryData';

export const MILITARY_FACTORY: PurposeSpendingKind<MilitaryFactorySpendingReply> = {
  product: 'military-factory-spending', name: 'Military factory spending', measure: 'military factory IC',
  root: { id: 'all', label: 'All military factory IC' }, bands: BANDS, options: OPTIONS,
  params: options => ({ equipment: options.equipment === 'land' ? 'land' : 'air' }),
  validate: validateFactorySpendingReply, items: factoryItems,
  columns: (data, decimals) => factoryColumns(data.months, decimals),
  month: (data, index) => {
    const month = data.months[index];
    return { firstHour: month.firstHour, lastHour: month.lastHour, note: factoryMonthNote(month),
      bands: [month.new, month.converted, month.unrecorded, month.other].map(exactOf), total: factoryMonthTotal(month) };
  },
  monthRow: (data, index) => {
    const month = data.months[index];
    return { path: [String(month.firstHour)], new: month.new.totalRaw, converted: month.converted.totalRaw,
      unrecorded: month.unrecorded.totalRaw, other: month.other.totalRaw,
      purposeUnproven: month.purposeUnproven, unresolved: month.unresolved };
  },
};

export default function MilitaryFactorySpendingWidget(props: WidgetProps) {
  return <PurposeSpendingWidget kind={MILITARY_FACTORY} {...props} />;
}
