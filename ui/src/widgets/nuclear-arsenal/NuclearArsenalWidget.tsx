import { useCampaignGate } from '../../dashboard/regionGate';
import { useCallback, useMemo } from 'react';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { RaceView } from '../../charts/race/options';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { OPTIONS } from './definition';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { arsenalStructure, nukeParts, validateNuclearArsenal } from './nuclearArsenalData.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const TOTAL = { kind: 'country-nuke-totals', measure: 'total' };
const format = (raw: string, decimals: number) => formatRaw(raw, decimals);
const formatValue = (value: number) => Math.round(value).toLocaleString('en-US');

export default function NuclearArsenalWidget({ widget, runtime, active, onDiagnostics, countrySlot }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view = (['race', 'lines', 'mix'].includes(String(options.view)) ? options.view : 'race') as RaceView;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const race = view === 'race' && options.show !== 'breakdown';
  const end = useCampaignGate(runtime, widget, active && !race).hour;
  const names = useProduct(runtime.client, !race && track && end !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'nuclear-arsenal',
    params: { branchId: track.branch.id, hour: end } }) : null, active, validateNuclearArsenal, track?.source);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'nuclear-arsenal', race ? {} : null, { active: active && race, validate: validateNuclearArsenal, structure: arsenalStructure });
  const metadata = race ? head.data : names.data, shown = metadata;
  const parts = useMemo(() => nukeParts(metadata), [metadata]);
  const onWarnings = useCallback((warnings: readonly string[]) => {
    const all = [...warnings, ...(shown?.warnings ?? []), ...(!race && names.error ? [`Weapon type names could not be loaded: ${names.error.message}`] : [])];
    onDiagnostics(all.length ? all : NO_WARNINGS);
  }, [onDiagnostics, shown, names.error, race]);
  return <MeasureRace arsenal={race ? head : undefined} widget={widget} runtime={runtime} active={active} countrySlot={countrySlot} measure={TOTAL} decimals={0} view={view} options={options}
    label="Nuclear stockpile" icon="target" format={format} formatValue={formatValue} onWarnings={onWarnings} parts={parts}
    split={options.split !== false} positiveOnly />;
}
