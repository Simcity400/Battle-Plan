import { useCallback } from 'react';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';

const NO_WARNINGS: string[] = [];
const CONTRIBUTIONS = { kind: 'threat-contributions', measure: 'contribution' };
const formatContribution = (raw: string, decimals: number) => `${formatRaw(raw, decimals, 2)}%`;
const formatPercent = (value: number) => `${Math.round(value * 10) / 10}%`;

export default function WorldTensionWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const onWarnings = useCallback((warnings: readonly string[]) => props.onDiagnostics(warnings.length ? [...warnings] : NO_WARNINGS), [props.onDiagnostics]);
  return (
    <MeasureRace
      widget={props.widget}
      runtime={props.runtime}
      active={props.active}
      measure={CONTRIBUTIONS}
      decimals={5}
      view="race"
      options={options}
      label="Contribution to world tension"
      icon="alert"
      format={formatContribution}
      formatValue={formatPercent}
      onWarnings={onWarnings}
    />
  );
}
