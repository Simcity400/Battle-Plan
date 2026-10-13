import type { WidgetProps } from '../../dashboard/definitions';
import PoliticsSpendingTable from '../../dashboard/PoliticsSpendingTable';

export default function PoliticalPowerSpendingWidget(props: WidgetProps) {
  return <PoliticsSpendingTable {...props} currency="Political" />;
}
