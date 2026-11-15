import FactionTotalsView from '../../charts/race/FactionTotalsView';
import AggregateTotalsView, { aggregated } from '../../charts/race/AggregateTotalsView';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { formatTerritory } from '../../dashboard/territoryMeasure.ts';
import { OPTIONS } from './definition';
import { factionStrengthSource } from './factionStrengthData.ts';

export default function FactionStrengthWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const source = factionStrengthSource(options);
  // A group shown as one total: a faction's own, or any other group's members summed.
  const View = aggregated(props.widget.countries) ? AggregateTotalsView : FactionTotalsView;
  return <View {...props} options={options} kind={source.kind} measure={source.measure} label="Faction strength"
    measureLabel={source.label} note={source.note} format={formatTerritory} />;
}
