import type { WidgetProps } from '../../dashboard/definitions';
import PoliticsSpendingLog from '../../dashboard/PoliticsSpendingLog';

export default function PoliticalPowerSpendingWidget(props: WidgetProps) {
  return <PoliticsSpendingLog {...props} currency="Political" />;
}
