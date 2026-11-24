import { useCallback, useMemo, useRef, useState } from 'react';
import MeasureDetails from '../../charts/race/MeasureDetails';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { RaceView } from '../../charts/race/options';
import { dayLabel } from '../../charts/draw';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { contentSet } from '../../dashboard/options';
import { viewOptionsAsStored } from '../manifest.ts';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { parseChartInspect } from '../../families/time-chart/hitTest.ts';
import type { InstalledAction } from '../../families/plan.ts';
import type { FactoryPartsReply } from '../../generated/dashboard';
import { COUNT_MODES, countMode, factoryMeasure, factoryParts, partRows, validateFactoryGroups, validateFactoryParts } from './factoriesData.ts';
import type { FactoriesRowMeta } from './factoriesProjection.ts';

const NO_WARNINGS: string[] = [];
const format = (raw: string, decimals: number) => formatRaw(raw, decimals);
const formatValue = (value: number) => Math.round(value).toLocaleString('en-US');

function FactoryDetails({ widget, runtime, active, tag, hour, types, mode }: Pick<FamilyViewProps, 'widget' | 'runtime' | 'active'> & {
  tag: string; hour: GatedHour; types: readonly string[]; mode: ReturnType<typeof countMode>;
}) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'factory-parts',
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour },
  }) : null;
  const parts = useProduct<FactoryPartsReply>(runtime.client, request, active, validateFactoryParts, track?.source);
  return (
    <MeasureDetails
      title={`${tag}, ${dayLabel(hour)}`}
      rows={parts.data ? partRows(parts.data, types, mode) : undefined}
      error={parts.error}
      onRetry={parts.retry}
    />
  );
}

export default function FactoriesWidget(props: FamilyViewProps) {
  const { widget, runtime, active, onDiagnostics, countrySlot, manifestDeclaration, options } = props;
  const view = (['race', 'lines', 'mix'].includes(String(props.view)) ? props.view : 'race') as RaceView;
  const mode = countMode(options.count);
  const types = useMemo(() => contentSet(options.types), [options.types]);
  const chartWidget = useMemo(() => ({ ...widget, options: viewOptionsAsStored(widget.kind, props.view ?? view, options) }), [widget, props.view, view, options]);
  const isLines = manifestDeclaration?.family === 'time-chart';
  const familyDeclaration = manifestDeclaration && (isLines || options.show !== 'breakdown') ? manifestDeclaration : undefined;
  const isFamily = familyDeclaration !== undefined;

  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active && isFamily);

  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const named = useProduct(
    runtime.client,
    !isFamily && track
      ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'factory-groups', params: { branchId: track.branch.id } })
      : null,
    active && !isFamily,
    validateFactoryGroups,
    track?.source,
  );
  const measure = useMemo(() => ({ kind: 'country-factories', measure: factoryMeasure(mode, types) }), [mode, types]);
  const parts = useMemo(() => factoryParts(mode, types, named.data), [mode, types, named.data]);
  const onWarnings = useCallback((warnings: readonly string[]) => {
    const all = [...warnings, ...(named.data?.missing ? [named.data.missing] : []), ...(named.data?.warnings ?? [])];
    onDiagnostics(all.length ? all : NO_WARNINGS);
  }, [onDiagnostics, named.data]);

  const onActivate = useCallback((action: InstalledAction) => {
    if (isLines && parseChartInspect(action.subject) !== null) return;
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const rowMeta = action.meta as FactoriesRowMeta | null | undefined;
    setDetail({ tag: rowMeta?.tag ?? action.subject, key: rowMeta?.key ?? action.subject, hour: gated });
  }, [gate, isLines]);

  if (familyDeclaration) {
    return (
      <div ref={anchorRef} className="fh-frame">
        <FamilyWidget
          {...props}
          family={familyDeclaration.family}
          view={view}
          declaration={familyDeclaration}
          {...(isLines ? {} : { selectedKey: detail?.key ?? null })}
          onActivate={onActivate}
        />
        <DetailWindowHost
          open={detail !== null}
          subject={detail?.tag ?? null}
          hour={detail?.hour ?? null}
          title={detail ? `${detail.tag} factories` : 'Factories'}
          anchor={anchorRef}
          returnFocus=".fh-body"
          onClose={() => setDetail(null)}
        >
          {detail ? (
            <FactoryDetails
              widget={widget}
              runtime={runtime}
              active={active}
              tag={detail.tag}
              hour={detail.hour}
              types={types}
              mode={mode}
            />
          ) : null}
        </DetailWindowHost>
      </div>
    );
  }

  const label = `Factories, ${COUNT_MODES.find(item => item.value === mode)!.label.toLowerCase()}`;
  return (
    <MeasureRace
      widget={chartWidget}
      runtime={runtime}
      active={active}
      countrySlot={countrySlot}
      measure={measure}
      decimals={0}
      view={view}
      options={options}
      label={label}
      icon="factory"
      format={format}
      formatValue={formatValue}
      onWarnings={onWarnings}
      parts={parts}
      split={options.split === true}
      details={(tag, _hour, following, at) => (following || at === null
        ? null
        : <FactoryDetails widget={widget} runtime={runtime} active={active} tag={tag} hour={at} types={types} mode={mode} />)}
    />
  );
}
