import { useCampaignGate } from '../../dashboard/regionGate';
import { useCallback, useMemo } from 'react';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { OPTIONS } from './definition';
import { nukeParts, validateNuclearArsenal } from './nuclearArsenalData.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const TOTAL = { kind: 'country-nuke-totals', measure: 'total' };
const format = (raw: string, decimals: number) => formatRaw(raw, decimals);
const formatValue = (value: number) => Math.round(value).toLocaleString('en-US');

export default function NuclearArsenalWidget({ widget, runtime, active, onDiagnostics, countrySlot }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view: 'lines' | 'mix' = options.view === 'mix' ? 'mix' : 'lines';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const end = useCampaignGate(runtime, widget, active).hour;
  const names = useProduct(runtime.client, track && end !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'nuclear-arsenal',
    params: { branchId: track.branch.id, hour: end } }) : null, active, validateNuclearArsenal, track?.source);
  const parts = useMemo(() => nukeParts(names.data), [names.data]);
  const onWarnings = useCallback((warnings: readonly string[]) => {
    const all = [...warnings, ...(names.data?.warnings ?? []), ...(names.error ? [`Weapon type names could not be loaded: ${names.error.message}`] : [])];
    onDiagnostics(all.length ? all : NO_WARNINGS);
  }, [onDiagnostics, names.data, names.error]);
  return <MeasureRace widget={widget} runtime={runtime} active={active} countrySlot={countrySlot} measure={TOTAL} decimals={0} view={view} options={options}
    label="Nuclear stockpile" icon="target" format={format} formatValue={formatValue} onWarnings={onWarnings} parts={parts}
    split={options.split !== false} positiveOnly />;
}
