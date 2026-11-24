import MilitaryAirView from '../../dashboard/MilitaryAirView';
import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';

export default function AirKillsLossesWidget(props: WidgetProps) {
  return <MilitaryAirView {...props} family="airLosses" fields={OPTIONS} title="Air kills & losses" />;
}
