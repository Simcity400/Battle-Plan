import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import DrillHost from '../../charts/drill/DrillHost';
import Table, { CellBar } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { usNumber } from '../../charts/numberFormat';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { jumpItem } from '../../charts/jump';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useCountryNameAt, useReachedCountry, useReachedProduct } from '../../dashboard/reachedProduct';
import ReachedCountryChoice from '../../dashboard/ReachedCountryChoice';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS, VIEWS } from './definition';
import { divisionName, fill, items, percent, share, templateName, templateTooltip, typeFill, typeKey, typeName, typeTooltip, validateDivisionEquipment } from './divisionEquipmentData';
import type { DivisionPoolType, EquipmentDivision, EquipmentTemplate, EquipmentType } from './divisionEquipmentData';
import './divisionEquipment.css';

type Drill = { kind: 'type'; key: string } | { kind: 'template'; template: string } | { kind: 'division'; division: string; template: string | null };
const templateKey = (row: EquipmentTemplate) => row.template;
const divisionKey = (row: EquipmentDivision) => row.division;
const poolKey = typeKey;
const MISSING_SORT = { column: 'missing', direction: 'descending' } as const;
const FILL_SORT = { column: 'fill', direction: 'ascending' } as const;
const big = (raw: string | null) => raw === null ? null : BigInt(raw);

function Ratio({ ratio, label, bars }: { ratio: number | null; label: string; bars: boolean }) {
  return ratio === null ? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span>
    : <CellBar value={bars ? ratio : null} maximum={1}><span aria-label={label}>{percent(ratio)}</span></CellBar>;
}
function Missing({ raw }: { raw: string | null }) {
  if (raw === null) return <span className="division-equipment-missing" data-tooltip="Not recorded">—</span>;
  return BigInt(raw) > 0n ? <b className="division-equipment-short">{items(raw)}</b> : <>{items(raw)}</>;
}

export default function DivisionEquipmentWidget({ widget, runtime, active, setOption, onDiagnostics, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const [drill, setDrill] = useState<Drill | null>(null);
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const country = useReachedCountry(widget, runtime, hour, active, widget.options?.country, setOption);
  const tag = country.tag;
  const countryName = useCountryNameAt(runtime.client, track, tag ?? country.chosen, hour, active);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'template' ? 'template' : 'equipment';
  const countFor = options.countFor === 'host' ? 'host' : 'owner';
  const bars = options.cellBars === true;
  const subject = JSON.stringify([track?.branch.id, country.chosen, countFor]);
  const base = track && window && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'division-equipment' } : null;
  const state = useReachedProduct(runtime.client, track?.source, base && { ...base, params: { branchId: track!.branch.id, hour, tag, countFor } },
    subject, active, validateDivisionEquipment);
  const drillParams = drill?.kind === 'template' ? { template: drill.template } : drill?.kind === 'division' ? { division: drill.division } : null;
  const detail = useReachedProduct(runtime.client, track?.source, base && drillParams && { ...base, params: { branchId: track!.branch.id, hour, tag, countFor, ...drillParams } },
    JSON.stringify([subject, drillParams]), active && drillParams !== null, validateDivisionEquipment);
  const data = state.data;
  const viewportHeight = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  useEffect(() => setDrill(null), [subject, view]);
  const typeColumns = useMemo<readonly TableColumn<EquipmentType>[]>(() => [
    { key: 'type', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 2fr)', value: typeName,
      render: row => <span className="division-equipment-name" data-tooltip={row.archetype ?? undefined}><ArtFallback icon="hammer" /><span>{typeName(row)}</span></span> },
    { key: 'held', header: 'In divisions', label: 'In divisions', numeric: true, width: 'max-content', priority: 3, value: row => BigInt(row.heldRaw), render: row => items(row.heldRaw) },
    { key: 'needed', header: 'Needed', label: 'Needed', numeric: true, width: 'max-content', priority: 2, value: row => big(row.neededRaw), renderMissing: true,
      text: row => items(row.neededRaw) ?? 'Not recorded', render: row => items(row.neededRaw) ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
    { key: 'missing', header: 'Missing', label: 'Missing', numeric: true, width: 'max-content', value: row => big(row.missingRaw), renderMissing: true,
      text: row => items(row.missingRaw) ?? 'Not recorded', render: row => <Missing raw={row.missingRaw} /> },
    { key: 'fill', header: 'Fill', label: 'Fill', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', value: typeFill, renderMissing: true,
      text: row => { const ratio = typeFill(row); return ratio === null ? 'Not recorded' : percent(ratio); },
      render: row => <Ratio ratio={typeFill(row)} label="Fill" bars={bars} /> },
    { key: 'short', header: 'Divisions short', label: 'Divisions short', numeric: true, width: 'max-content', priority: 1, value: row => row.divisionsShort },
  ], [bars]);
  const templateColumns = useMemo<readonly TableColumn<EquipmentTemplate>[]>(() => [
    { key: 'template', header: 'Template', label: 'Template', width: 'minmax(var(--table-name-min), 2fr)', value: templateName,
      render: row => <span className="division-equipment-name" data-tooltip={row.template}>{row.icon ? <img src={row.icon} alt="" /> : <ArtFallback icon="helmet" />}<span>{templateName(row)}</span></span> },
    { key: 'divisions', header: 'Divisions', label: 'Divisions', numeric: true, width: 'max-content', priority: 3, value: row => row.divisions },
    { key: 'manpower', header: 'Manpower', label: 'Manpower', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 2, value: row => share(row.manpower, row.manpowerRequired), renderMissing: true,
      text: row => row.manpower === null ? 'Not recorded' : usNumber.format(Number(row.manpower)), render: row => <Ratio ratio={share(row.manpower, row.manpowerRequired)} label="Manpower" bars={bars} /> },
    { key: 'strength', header: 'Strength', label: 'Strength', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 4, value: row => share(row.hpRaw, row.hpMaxRaw), renderMissing: true,
      render: row => <Ratio ratio={share(row.hpRaw, row.hpMaxRaw)} label="Strength" bars={bars} /> },
    { key: 'fill', header: 'Equipment fill', label: 'Equipment fill', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', value: row => fill(row.heldRaw, row.neededRaw), renderMissing: true,
      render: row => <Ratio ratio={fill(row.heldRaw, row.neededRaw)} label="Equipment fill" bars={bars} /> },
    { key: 'short', header: 'Types short', label: 'Types short', numeric: true, width: 'max-content', priority: 1, value: row => row.typesShort },
  ], [bars]);
  const divisionColumns = useMemo<readonly TableColumn<EquipmentDivision>[]>(() => [
    { key: 'division', header: 'Division', label: 'Division', width: 'minmax(var(--table-name-min), 2fr)', value: divisionName,
      render: row => <span className="division-equipment-name" data-tooltip={row.division}><span>{divisionName(row)}</span></span> },
    { key: 'manpower', header: 'Manpower', label: 'Manpower', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 2, value: row => share(row.manpower, row.manpowerRequired), renderMissing: true,
      render: row => <Ratio ratio={share(row.manpower, row.manpowerRequired)} label="Manpower" bars={bars} /> },
    { key: 'strength', header: 'Strength', label: 'Strength', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 3, value: row => share(row.hpRaw, row.hpMaxRaw), renderMissing: true,
      render: row => <Ratio ratio={share(row.hpRaw, row.hpMaxRaw)} label="Strength" bars={bars} /> },
    { key: 'fill', header: 'Equipment fill', label: 'Equipment fill', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', value: row => row.poolKnown ? fill(row.heldRaw, row.neededRaw) : null, renderMissing: true,
      render: row => <Ratio ratio={row.poolKnown ? fill(row.heldRaw, row.neededRaw) : null} label="Equipment fill" bars={bars} /> },
    { key: 'short', header: 'Types short', label: 'Types short', numeric: true, width: 'max-content', priority: 1, value: row => row.poolKnown ? row.typesShort : null },
  ], [bars]);
  const poolColumns = useMemo<readonly TableColumn<DivisionPoolType>[]>(() => [
    { key: 'type', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 2fr)', value: typeName, render: row => <span data-tooltip={row.archetype ?? undefined}>{typeName(row)}</span> },
    { key: 'held', header: 'In division', label: 'In division', numeric: true, width: 'max-content', priority: 2, value: row => BigInt(row.heldRaw), render: row => items(row.heldRaw) },
    { key: 'needed', header: 'Needed', label: 'Needed', numeric: true, width: 'max-content', priority: 1, value: row => big(row.neededRaw), renderMissing: true,
      render: row => items(row.neededRaw) ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
    { key: 'missing', header: 'Missing', label: 'Missing', numeric: true, width: 'max-content', value: row => big(row.missingRaw), renderMissing: true, render: row => <Missing raw={row.missingRaw} /> },
  ], []);
  const typeTip = useCallback((row: EquipmentType) => data ? fitTooltip([typeTooltip(row, data)], viewportHeight) : { title: typeName(row), rows: [] }, [data, viewportHeight]);
  const templateTip = useCallback((row: EquipmentTemplate) => data ? fitTooltip([templateTooltip(row, data)], viewportHeight) : { title: templateName(row), rows: [] }, [data, viewportHeight]);
  const menu = () => data?.poolHour == null || !track ? [] : [jumpItem(runtime, track.id, data.poolHour)];
  const short = data?.divisionsShort ?? 0;
  const unknownShort = data?.unknownDivisions ? `${usNumber.format(data.unknownDivisions)} ${data.unknownDivisions === 1 ? 'division' : 'divisions'} not recorded` : undefined;
  const header = <>
    {headerSlot && setOption && createPortal(<Segmented value={view} options={VIEWS} label="Division equipment view" onChange={value => setOption('view', value)} />, headerSlot)}
    {headerSlot && data && !data.absent && createPortal(<span className="pill" data-tooltip={unknownShort}>{usNumber.format(short)} {short === 1 ? 'division' : 'divisions'} short</span>, headerSlot)}
  </>;
  let main;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see division equipment." />;
  else if (!window) main = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (!country.chosen) main = <WidgetState kind="empty" message="Choose a country to see its division equipment." />;
  else if (country.error) main = <WidgetState kind="error" detail={country.error} onRetry={country.retry} />;
  else if (country.holders) main = <ReachedCountryChoice tag={country.chosen} holders={country.holders} onPick={country.pick} />;
  else if (country.reason) main = <WidgetState kind="unavailable" message={country.reason} />;
  else if (country.absent) main = <WidgetState kind="unavailable" message={`${countryName} does not exist at this date.`} />;
  else if (!tag) main = <WidgetState kind="loading" label="Loading division equipment…" />;
  else if (state.error) main = <WidgetState kind="error" detail={state.error} onRetry={state.retry} />;
  else if (!data) main = <WidgetState kind="loading" label="Loading division equipment…" />;
  else if (data.absent) main = <WidgetState kind="unavailable" message={`${countryName} does not exist at this date.`} />;
  else if (!data.divisions) main = <WidgetState kind="empty" message={`${countryName} has no divisions at this date.`} />;
  else main = <>{data.poolHour !== null && <p className="division-equipment-caption">Equipment as of {formatDateTime(data.poolHour)}{data.unknownDivisions > 0
      ? ` · ${usNumber.format(data.unknownDivisions)} ${data.unknownDivisions === 1 ? 'division' : 'divisions'} not recorded` : ''}</p>}
    {view === 'equipment'
      ? <Table label="Division equipment by type" rows={data.types} columns={typeColumns} rowKey={typeKey} sort={MISSING_SORT} setOption={setOption}
        cellBars={bars} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        onRowClick={row => setDrill({ kind: 'type', key: typeKey(row) })} tooltip={typeTip} menu={menu} />
      : <Table label="Division equipment by template" rows={data.templates} columns={templateColumns} rowKey={templateKey} sort={FILL_SORT} setOption={setOption}
        cellBars={bars} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        onRowClick={row => setDrill({ kind: 'template', template: row.template })} tooltip={templateTip} menu={menu} />}</>;
  const type = drill?.kind === 'type' ? data?.types.find(row => typeKey(row) === drill.key) : undefined;
  const drillData = detail.data;
  let body = null;
  if (drill?.kind === 'type') body = type ? <div className="division-equipment-drill">
    <Table label={`${typeName(type)} models`} rows={type.variants} columns={[
      { key: 'model', header: 'Model', label: 'Model', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? 'Design name unavailable',
        render: row => <span data-tooltip={row.variant}>{row.name ?? 'Design name unavailable'}</span> },
      { key: 'held', header: 'In divisions', label: 'In divisions', numeric: true, width: 'max-content', bar: 'visible', value: row => BigInt(row.heldRaw), render: row => items(row.heldRaw) },
    ]} rowKey={row => row.variant} sort={{ column: 'held', direction: 'descending' }} cellBars={bars} narrow={narrowTableOf(options)} menu={menu} /></div>
    : <WidgetState kind="unavailable" message="This equipment type is not held at this date." />;
  else if (drill && detail.error) body = <WidgetState kind="error" detail={detail.error} onRetry={detail.retry} />;
  else if (drill && !drillData) body = <WidgetState kind="loading" label="Loading…" />;
  else if (drill?.kind === 'template' && drillData) body = <div className="division-equipment-drill">
    <Table label="Divisions of this template" rows={drillData.divisionRows ?? []} columns={divisionColumns} rowKey={divisionKey} sort={FILL_SORT}
      cellBars={bars} narrow={narrowTableOf(options)} onRowClick={row => setDrill({ kind: 'division', division: row.division, template: drill.template })}
      tooltip={row => ({ title: divisionName(row), rows: [
        { label: 'Equipment', value: row.poolKnown ? `${items(row.heldRaw) ?? '—'} of ${items(row.neededRaw) ?? '—'}` : 'Not recorded' },
        { label: 'Types short', value: row.poolKnown ? usNumber.format(row.typesShort) : 'Not recorded' },
        ...(row.convertingFrom ? [{ label: 'Converting from', value: row.convertingFrom }] : []),
      ] })} menu={menu} /></div>;
  else if (drill?.kind === 'division' && drillData) body = drillData.pool ? <div className="division-equipment-drill">
    <p><b>{divisionName(drillData.pool)}</b> · {drillData.pool.templateName ?? 'Template name unavailable'}</p>
    <Table label="Equipment in this division" rows={drillData.pool.types} columns={poolColumns} rowKey={poolKey} sort={MISSING_SORT}
      cellBars={bars} narrow={narrowTableOf(options)} menu={menu}
      tooltip={row => ({ title: typeName(row), rows: row.variants.map(variant => ({ label: variant.name ?? 'Design name unavailable', value: items(variant.heldRaw) ?? '—' })) })} /></div>
    : <WidgetState kind="unavailable" message="This division is not in service at this date." />;
  const back = drill?.kind === 'division' && drill.template ? { kind: 'template' as const, template: drill.template } : null;
  return <div ref={anchor} className="division-equipment">{header}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill !== null}
      title={drill?.kind === 'type' ? type ? typeName(type) : 'Equipment' : drill?.kind === 'template'
        ? templateName(data?.templates.find(row => row.template === drill.template) ?? { name: null }) : 'Division'}
      day={formatDateTime(hour)} following={widget.time.mode === 'follow'} canFollow={false} anchor={anchor}
      onClose={() => setDrill(back)} onFollow={() => {}} main={main}>{body}</DrillHost></div>;
}
