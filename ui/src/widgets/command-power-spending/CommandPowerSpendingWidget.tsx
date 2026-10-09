import type { WidgetProps } from '../../dashboard/definitions';
import PoliticsSpendingLog from '../../dashboard/PoliticsSpendingLog';

export default function CommandPowerSpendingWidget(props: WidgetProps) {
  return <PoliticsSpendingLog {...props} currency="Command" />;
}
