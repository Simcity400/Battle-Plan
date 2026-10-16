import type { WidgetProps } from '../../dashboard/definitions';
import { HistoryEquipmentProducedWidget } from './EquipmentProducedWidget';

/** Endpoint pages use the same caption, race and breakdown as the history view. */
export default function PreparedEquipmentProducedWidget(props: WidgetProps) {
  return <HistoryEquipmentProducedWidget {...props} prepared />;
}
