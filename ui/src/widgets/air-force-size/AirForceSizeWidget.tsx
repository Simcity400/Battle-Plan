import PartsRaceView from '../../charts/race/PartsRaceView';
import type { PartsRaceViewConfig } from '../../charts/race/PartsRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { OPTIONS } from './definition';
import { airForceRequest, validateAirForceReply } from './airForceData';

const config: PartsRaceViewConfig = {
  product: 'air-force-size', title: 'Air force size', subject: 'air forces', icon: 'layers',
  options: OPTIONS, request: airForceRequest, validate: validateAirForceReply,
  measure: options => options.measure === 'wings' ? 'Number of wings' : 'Number of planes',
  ownership: options => options.ownership === 'effective' ? 'Country it fights for' : 'Roster owner',
  partsLabel: () => 'Equipment archetypes',
  resetKey: options => [options.measure, options.ownership],
  zeroMix: options => `The recorded ${options.measure === 'wings' ? 'wing' : 'plane'} total is zero; there are no equipment types to mix.`,
  note: options => options.measure === 'wings' ? 'Each recorded wing counts once; wings with multiple types are shown as Mixed types.' : 'Planes in wings; stockpiled planes are not included.',
};

export default function AirForceSizeWidget(props: WidgetProps) {
  return <PartsRaceView {...props} config={config} />;
}
