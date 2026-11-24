import { useCallback, useMemo } from 'react';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';
import { SCORE_LABELS, scorePart } from './gameScoreData.ts';

const NO_WARNINGS: string[] = [];
const format = (raw: string, decimals: number) => formatRaw(raw, decimals);
const formatValue = (value: number) => Math.round(value).toLocaleString('en-US');

export default function GameScoreWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const by = scorePart(options.topMeasure);
  const measure = useMemo(() => ({ kind: 'country-game-scores', measure: by }), [by]);
  const onWarnings = useCallback((warnings: readonly string[]) => props.onDiagnostics(warnings.length ? [...warnings] : NO_WARNINGS), [props.onDiagnostics]);
  return <MeasureRace widget={props.widget} runtime={props.runtime} active={props.active} measure={measure} decimals={5} view="lines" options={options}
    label={SCORE_LABELS[by]} icon="grid" format={format} formatValue={formatValue} onWarnings={onWarnings} breakdown={false} />;
}
