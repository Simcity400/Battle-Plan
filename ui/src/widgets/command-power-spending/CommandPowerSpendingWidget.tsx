import type { WidgetProps } from '../../dashboard/definitions';
import PoliticsSpendingTable from '../../dashboard/PoliticsSpendingTable';

export default function CommandPowerSpendingWidget(props: WidgetProps) {
  return <PoliticsSpendingTable {...props} currency="Command" />;
}
