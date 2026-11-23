import type { GatedHour } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo } from 'react';
import MeasureRace from '../../charts/race/MeasureRace';
import MeasureDetails from '../../charts/race/MeasureDetails';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { RaceView } from '../../charts/race/options';
import { dayLabel } from '../../charts/draw';
import type { WidgetProps } from '../../dashboard/definitions';
import { contentSet, resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { OPTIONS } from './definition';
import { COUNT_MODES, countMode, factoryMeasure, factoryParts, partRows, validateFactoryGroups, validateFactoryParts } from './factoriesData.ts';
import type { FactoryPartsReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const format = (raw: string, decimals: number) => formatRaw(raw, decimals);
const formatValue = (value: number) => Math.round(value).toLocaleString('en-US');

function FactoryDetails({ widget, runtime, active, tag, hour, types, mode }: Pick<WidgetProps, 'widget' | 'runtime' | 'active'> & {
  tag: string; hour: GatedHour; types: readonly string[]; mode: ReturnType<typeof countMode> }) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'factory-parts',
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour } }) : null;
  const parts = useProduct<FactoryPartsReply>(runtime.client, request, active, validateFactoryParts, track?.source);
  return <MeasureDetails title={`${tag}, ${dayLabel(hour)}`} rows={parts.data ? partRows(parts.data, types, mode) : undefined}
    error={parts.error} onRetry={parts.retry} />;
}

export default function FactoriesWidget({ widget, runtime, active, onDiagnostics, countrySlot }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view = (['race', 'lines', 'mix'].includes(String(options.view)) ? options.view : 'race') as RaceView;
  const mode = countMode(options.count);
  const types = useMemo(() => contentSet(options.types), [options.types]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const named = useProduct(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'factory-groups',
    params: { branchId: track.branch.id } }) : null, active, validateFactoryGroups, track?.source);
  const measure = useMemo(() => ({ kind: 'country-factories', measure: factoryMeasure(mode, types) }), [mode, types]);
  const parts = useMemo(() => factoryParts(mode, types, named.data), [mode, types, named.data]);
  const onWarnings = useCallback((warnings: readonly string[]) => {
    const all = [...warnings, ...(named.data?.missing ? [named.data.missing] : []), ...(named.data?.warnings ?? [])];
    onDiagnostics(all.length ? all : NO_WARNINGS);
  }, [onDiagnostics, named.data]);
  const label = `Factories, ${COUNT_MODES.find(item => item.value === mode)!.label.toLowerCase()}`;
  return <MeasureRace widget={widget} runtime={runtime} active={active} countrySlot={countrySlot} measure={measure} decimals={0} view={view} options={options}
    label={label} icon="factory" format={format} formatValue={formatValue} onWarnings={onWarnings} parts={parts} split={options.split === true}
    details={(tag, _hour, following, at) => following || at === null ? null
      : <FactoryDetails widget={widget} runtime={runtime} active={active} tag={tag} hour={at} types={types} mode={mode} />} />;
}
