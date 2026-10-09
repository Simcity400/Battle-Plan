import PartsRaceView from '../../charts/race/PartsRaceView';
import type { PartsRaceViewConfig } from '../../charts/race/PartsRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';
import { formatHullTick, formatNavyValue, navyChartOptions, navyRequest, validateNavyReply } from './navyData';

const config: PartsRaceViewConfig = {
  product: 'navy-size', title: 'Navy size', subject: 'navies', icon: 'flows',
  options: OPTIONS, request: navyRequest, validate: validateNavyReply,
  measure: options => options.measure === 'strength' ? 'Current hull strength (HP)' : 'Number of ships',
  ownership: options => options.ownership === 'logical' ? 'Logical country' : 'Roster owner',
  partsLabel: options => options.grouping === 'class' ? 'Authored ship classes' : 'Broad roles',
  resetKey: options => [options.measure, options.ownership, options.grouping],
  chartOptions: navyChartOptions,
  zeroMix: options => `${options.measure === 'strength' ? 'Current hull HP' : 'The recorded ship count'} is zero; there are no ${options.grouping === 'type' ? 'broad roles' : 'ship classes'} to mix.`,
  formatValue: formatNavyValue,
  formatTick: options => options.measure === 'strength' ? formatHullTick : undefined,
  referenceLabel: options => options.measure === 'strength' ? 'Maximum hull strength (HP)' : undefined,
  note: options => options.measure === 'strength' ? 'Summed current hull HP; maximum HP is shown when known.' : 'Each recorded ship counts once.',
};

export default function NavySizeWidget(props: WidgetProps) {
  return <PartsRaceView {...props} config={config} />;
}
