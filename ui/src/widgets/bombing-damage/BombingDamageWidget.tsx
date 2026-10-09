import MilitaryAirView from '../../dashboard/MilitaryAirView';
import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';

export default function BombingDamageWidget(props: WidgetProps) {
  return <MilitaryAirView {...props} family="bombing" fields={OPTIONS} title="Bombing damage" />;
}
