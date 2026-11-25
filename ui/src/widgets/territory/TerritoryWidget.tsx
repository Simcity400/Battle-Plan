import { useCallback, useMemo } from 'react';
import FactionTotalsView from '../../charts/race/FactionTotalsView';
import AggregateTotalsView, { aggregated } from '../../charts/race/AggregateTotalsView';
import MeasureRace from '../../charts/race/MeasureRace';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import type { OptionValue } from '../../dashboard/options';
import { formatTerritory, measureOf, ruleOf, territoryMeasureLabel, territoryNote, territorySeriesMeasure } from '../../dashboard/territoryMeasure.ts';
import type { TerritoryMeasure, TerritoryRule } from '../../dashboard/territoryMeasure';
import { OPTIONS } from './definition';
import '../../charts/race/countryRace.css';

const NO_WARNINGS: string[] = [];

export default function TerritoryWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const measure = measureOf(options.measure), rule = ruleOf(options.rule);
  if (props.widget.countries.kind === 'factions') return <FactionTotalsView {...props} options={options} kind="territory"
    measure={territorySeriesMeasure(measure, rule)} label="Territory" measureLabel={territoryMeasureLabel(measure, rule)}
    note={territoryNote(measure)} format={formatTerritory} />;
  if (aggregated(props.widget.countries)) return <AggregateTotalsView {...props} options={options} kind="territory"
    measure={territorySeriesMeasure(measure, rule)} label="Territory" measureLabel={territoryMeasureLabel(measure, rule)}
    note={territoryNote(measure)} format={formatTerritory} />;
  return <TerritoryRaceView {...props} options={options} measure={measure} rule={rule} />;
}

interface Chosen { measure: TerritoryMeasure; rule: TerritoryRule }

const formatRaw = (raw: string) => formatTerritory(Number(raw));

function TerritoryRaceView({ widget, runtime, active, onDiagnostics, options, measure, rule }: WidgetProps & Chosen & { options: Readonly<Record<string, OptionValue>> }) {
  const family = useMemo(() => ({ kind: 'territory', measure: territorySeriesMeasure(measure, rule) }), [measure, rule]);
  const onWarnings = useCallback((warnings: readonly string[]) => onDiagnostics(warnings.length ? [...warnings] : NO_WARNINGS), [onDiagnostics]);
  return <MeasureRace widget={widget} runtime={runtime} active={active} measure={family} decimals={0} view="lines"
    options={options} label={`Territory, ${territoryMeasureLabel(measure, rule)}`} icon="map" format={formatRaw} formatValue={formatTerritory}
    note={territoryNote(measure)} onWarnings={onWarnings} />;
}
