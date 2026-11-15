import type { WidgetProps } from '../../dashboard/definitions';
import PowerSpendingTable from '../../dashboard/PowerSpendingTable';

export default function CommandPowerSpendingWidget(props: WidgetProps) {
  return <PowerSpendingTable {...props} currency="Command" />;
}
