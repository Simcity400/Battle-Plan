import NavyManagement from './NavyManagement';
import PartsRaceView from '../../charts/race/PartsRaceView';
import type { PartsRaceViewConfig } from '../../charts/race/PartsRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';
import { formatHullTick, formatNavyValue, navyChartOptions, navyRequest, validateNavyReply } from './navyData';

const config: PartsRaceViewConfig = {
  details: NavyManagement,
  product: 'navy-size', title: 'Navy size', subject: 'navies', icon: 'flows',
  options: OPTIONS, request: navyRequest, validate: validateNavyReply,
  measure: options => options.measure === 'strength' ? 'Current hull strength (HP)' : 'Number of ships',
  ownership: options => options.ownership === 'logical' ? 'Assigned country' : 'Country holding the fleet',
  partsLabel: options => options.grouping === 'class' ? 'Ship classes' : 'Roles',
  resetKey: options => [options.measure, options.ownership, options.grouping],
  chartOptions: navyChartOptions,
  zeroMix: options => options.measure === 'strength' ? 'No hull strength at this time.' : 'No ships at this time.',
  formatValue: formatNavyValue,
  formatTick: options => options.measure === 'strength' ? formatHullTick : undefined,
  referenceLabel: options => options.measure === 'strength' ? 'Maximum hull strength (HP)' : undefined,
};

export default function NavySizeWidget(props: WidgetProps) {
  return <PartsRaceView {...props} config={config} />;
}
