import PartsRaceView from '../../charts/race/PartsRaceView';
import type { PartsRaceViewConfig } from '../../charts/race/PartsRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';
import { formatNavalCount, navalMeasure, navalOwnership, navalPeriod, navalRequest } from './navalLossesData';

const config: PartsRaceViewConfig = {
  product: 'naval-losses', title: 'Naval losses', subject: 'naval losses', icon: 'shield',
  options: OPTIONS, request: navalRequest,
  measure: navalMeasure, ownership: navalOwnership,
  partsLabel: () => 'Ships and convoys',
  zeroMix: options => navalPeriod(options) === 'month' ? 'No known naval losses this month.' : 'No known naval losses yet.',
  formatValue: formatNavalCount,
  positiveTop: true,
  canFollow: options => navalPeriod(options) === 'running',
  note: () => 'Counted when the game lists a sinking; a country is named as it was when the sinking was read.',
};

export default function NavalLossesWidget(props: WidgetProps) {
  return <PartsRaceView {...props} config={config} />;
}
