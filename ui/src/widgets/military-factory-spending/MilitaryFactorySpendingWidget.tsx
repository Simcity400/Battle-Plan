import type { WidgetProps } from '../../dashboard/definitions';
import type { OptionValue } from '../../dashboard/options';
import { PurposeSpendingWidget } from '../dockyard-spending/PurposeSpending';
import type { PurposeSpendingKind } from '../dockyard-spending/PurposeSpending';
import { validateProductionSpendingReply } from '../production-spending/spendingData';
import { OPTIONS } from './definition';
import { BANDS, factoryItems, factoryMonth, factoryTotal } from './factoryData';

const domainOf = (options: Readonly<Record<string, OptionValue>>) => options.equipment === 'land' ? 'land' : 'air';

export const MILITARY_FACTORY: PurposeSpendingKind = {
  name: 'Military factory spending', measure: 'military factory IC',
  root: { id: 'all', label: 'All military factory IC' }, bands: BANDS, options: OPTIONS, repairs: false,
  validate: validateProductionSpendingReply,
  items: (data, options, window, colours) => factoryItems(data, domainOf(options), window, colours),
  total: (data, options, window) => factoryTotal(data, domainOf(options), window),
  month: (data, options, first, last, branchFirst) => factoryMonth(data, domainOf(options), first, last, branchFirst),
};

export default function MilitaryFactorySpendingWidget(props: WidgetProps) {
  return <PurposeSpendingWidget kind={MILITARY_FACTORY} {...props} />;
}
