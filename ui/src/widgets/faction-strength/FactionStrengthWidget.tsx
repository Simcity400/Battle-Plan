import FactionRaceView from '../../charts/race/FactionRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { formatTerritory, MEASURES as TERRITORY_MEASURES } from '../../dashboard/territoryMeasure';
import { definition, OPTIONS } from './definition';
import { factionStrengthLabel, factionStrengthNote, factionStrengthSource } from './factionStrengthData';

export default function FactionStrengthWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const source = factionStrengthSource(options);
  const unavailable = source.kind === 'territory' && options.territoryMeasure
    && !TERRITORY_MEASURES.some(item => item.value === options.territoryMeasure);
  return <FactionRaceView {...props} options={options} source={source} max={definition.countries.max} label="Faction strength"
    measureLabel={unit => factionStrengthLabel(source, unit)} formatValue={formatTerritory} note={factionStrengthNote(source)}
    unavailableReason={unavailable ? 'This territory measure is unavailable.' : undefined} />;
}
