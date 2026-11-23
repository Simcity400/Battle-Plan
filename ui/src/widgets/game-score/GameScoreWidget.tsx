import { useCallback, useMemo } from 'react';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import DeclaredTableWidget from '../../families/table/DeclaredTableWidget';
import { OPTIONS } from './definition';
import { SCORE_LABELS, scorePart, scoreParts } from './gameScoreData.ts';
import { gameScoreDeclaration } from './gameScoreProjection.ts';

const NO_WARNINGS: string[] = [];
const format = (raw: string, decimals: number) => formatRaw(raw, decimals);
const formatValue = (value: number) => Math.round(value).toLocaleString('en-US');
const SCORE_SPLIT = scoreParts();

export default function GameScoreWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const by = scorePart(options.topMeasure);
  const view = options.view === 'race' ? 'race' : options.view === 'lines' ? 'lines' : 'table';
  const measure = useMemo(() => ({ kind: 'country-game-scores', measure: by }), [by]);
  const onWarnings = useCallback((warnings: readonly string[]) => props.onDiagnostics(warnings.length ? [...warnings] : NO_WARNINGS), [props.onDiagnostics]);
  if (view === 'table') return <DeclaredTableWidget {...props} declaration={gameScoreDeclaration} />;
  return <MeasureRace widget={props.widget} runtime={props.runtime} active={props.active} measure={measure} decimals={5} view={view} options={options}
    label={SCORE_LABELS[by]} icon="grid" format={format} formatValue={formatValue} onWarnings={onWarnings}
    parts={by === 'total' ? SCORE_SPLIT : undefined} split={view === 'race' && options.split !== false} breakdown={false} />;
}
