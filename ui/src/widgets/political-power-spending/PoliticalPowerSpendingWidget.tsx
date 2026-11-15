import type { WidgetProps } from '../../dashboard/definitions';
import PowerSpendingTable from '../../dashboard/PowerSpendingTable';

export default function PoliticalPowerSpendingWidget(props: WidgetProps) {
  return <PowerSpendingTable {...props} currency="Political" />;
}
