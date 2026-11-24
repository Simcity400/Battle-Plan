import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import Table, { CellBar } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { usNumber } from '../../charts/numberFormat';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { jumpItem } from '../../charts/jump';
import { formatDateTime } from '../../dashboard/calendar';
import { contentTitleHint } from '../../dashboard/contentTitles';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import GateState from '../../dashboard/GateState';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS } from './definition';
import { counted, divisionName, equipmentStructure, fill, items, percent, share, templateName, templateTooltip, typeFill, typeKey, typeName, typeTooltip, validateDivisionEquipment } from './divisionEquipmentData';
import './divisionEquipment.css';
import type { DivisionPoolType, EquipmentDivision, EquipmentTemplate, EquipmentType } from '../../generated/dashboard';
import { SPEC } from '../../dashboard/regionTime';
import { useRegion } from '../../dashboard/widgetTime';

type Drill = { kind: 'type'; key: string } | { kind: 'template'; template: string } | { kind: 'division'; division: string; template: string | null };
const templateKey = (row: EquipmentTemplate) => row.template;
const divisionKey = (row: EquipmentDivision) => row.division;
const poolKey = typeKey;
const MISSING_SORT = { column: 'missing', direction: 'descending' } as const;
const FILL_SORT = { column: 'fill', direction: 'ascending' } as const;
const big = (raw: string | null) => raw === null ? null : BigInt(raw);

function ratioCell({ ratio, label, bars }: { ratio: number | null; label: string; bars: boolean }) {
  return ratio === null ? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span>
    : <CellBar value={bars ? ratio : null} maximum={1}><span aria-label={label}>{percent(ratio)}</span></CellBar>;
}
export function missingCell(raw: string | null) {
  if (raw === null) return <span className="division-equipment-missing" data-tooltip="Not recorded">—</span>;
  return BigInt(raw) > 0n ? <b className="division-equipment-short">{items(raw)}</b> : <>{items(raw)}</>;
}

export default function DivisionEquipmentWidget({ widget, runtime, active, setOption, onDiagnostics, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const [storedDrill, setStoredDrill] = useState<{ owner: object; drill: Drill } | null>(null);
  const mainRegion = useRegion(widget);
  const itemDrill = useDrill(runtime, mainRegion);
  const gate = useGate(runtime, widget, SPEC, active, { country: { stored: widget.options?.country, setOption } });
  const country = gate.country!;
  const hour = gate.hour;
  const tag = gate.open ? country.tag : null;
  const countryLabel = useCountryNameAt(runtime.client, track, tag ?? country.chosen, hour, active);
  const countryName = countryLabel.name;
  const nameDetail = countryLabel.nameFailure ? <span data-detail={countryLabel.nameFailure.message}>{countryName}</span> : undefined;
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'template' ? 'template' : 'equipment';
  const countFor = options.countFor === 'host' ? 'host' : 'owner';
  const bars = options.cellBars === true;
  const subject = JSON.stringify([track?.branch.id, country.chosen, countFor]);
  const asked = tag ? { country: countryFromKey(tag), countFor } : null;
  const state = useHeadProduct(runtime, gate, 'division-equipment', asked, { active, validate: validateDivisionEquipment, structure: equipmentStructure });
  useShownEachFrame(runtime.clock, state.shownAt, state.live);
  const data = state.data;
  const drillOwner = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, subject, view]);
  const drill = storedDrill?.owner === drillOwner ? storedDrill.drill : null;
  const setDrill = (next: Drill | null) => { setStoredDrill(next && { owner: drillOwner, drill: next }); if (next) itemDrill.attach(JSON.stringify(next)); else itemDrill.close(); };
  const drillParams = drill?.kind === 'template' ? { template: drill.template } : drill?.kind === 'division' ? { division: drill.division } : null;
  const detail = useHeadProduct(runtime, gate, 'division-equipment', asked && drillParams && { ...asked, ...drillParams },
    { active: active && drillParams !== null, validate: validateDivisionEquipment, structure: equipmentStructure });
  const viewportHeight = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  useDiagnostics(onDiagnostics, data ? [...new Set([...data.warnings, ...detail.data?.warnings ?? []])] : null);
  const typeColumns = useMemo<readonly TableColumn<EquipmentType>[]>(() => [
    { key: 'type', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 2fr)', value: typeName,
      render: row => <span className="division-equipment-name" data-tooltip={contentTitleHint(row.archetype)}><ArtFallback icon="hammer" name={typeName(row)} /><span>{typeName(row)}</span></span> },
    { key: 'held', header: 'In divisions', label: 'In divisions', numeric: true, width: 'max-content', priority: 3, value: row => big(row.heldRaw), renderMissing: true,
      text: row => items(row.heldRaw) ?? 'Not recorded', render: row => items(row.heldRaw) ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
    { key: 'needed', header: 'Needed', label: 'Needed', numeric: true, width: 'max-content', priority: 2, value: row => big(row.neededRaw), renderMissing: true,
      text: row => items(row.neededRaw) ?? 'Not recorded', render: row => items(row.neededRaw) ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
    { key: 'missing', header: 'Missing', label: 'Missing', numeric: true, width: 'max-content', value: row => big(row.missingRaw), renderMissing: true,
      text: row => items(row.missingRaw) ?? 'Not recorded', render: row => missingCell(row.missingRaw) },
    { key: 'fill', header: 'Fill', label: 'Fill', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', value: typeFill, renderMissing: true,
      text: row => { const ratio = typeFill(row); return ratio === null ? 'Not recorded' : percent(ratio); },
      render: row => ratioCell({ ratio: typeFill(row), label: 'Fill', bars }) },
    { key: 'short', header: 'Divisions short', label: 'Divisions short', numeric: true, width: 'max-content', priority: 1, value: row => row.divisionsShort, renderMissing: true,
      text: row => counted(row.divisionsShort), render: row => row.divisionsShort ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
  ], [bars]);
  const templateColumns = useMemo<readonly TableColumn<EquipmentTemplate>[]>(() => [
    { key: 'template', header: 'Template', label: 'Template', width: 'minmax(var(--table-name-min), 2fr)', value: templateName,
      render: row => <span className="division-equipment-name" title={templateName(row)}>{row.icon ? <img src={row.icon} alt="" /> : <ArtFallback icon="helmet" name={templateName(row)} reason={row.iconMissing} />}<span>{templateName(row)}</span></span> },
    { key: 'divisions', header: 'Divisions', label: 'Divisions', numeric: true, width: 'max-content', priority: 3, value: row => row.divisions, renderMissing: true,
      text: row => counted(row.divisions), render: row => row.divisions ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
    { key: 'manpower', header: 'Manpower', label: 'Manpower', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 2, value: row => share(row.manpower, row.manpowerRequired), renderMissing: true,
      text: row => row.manpower === null ? 'Not recorded' : usNumber.format(Number(row.manpower)), render: row => ratioCell({ ratio: share(row.manpower, row.manpowerRequired), label: 'Manpower', bars }) },
    { key: 'strength', header: 'Strength', label: 'Strength', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 4, value: row => share(row.hpRaw, row.hpMaxRaw), renderMissing: true,
      render: row => ratioCell({ ratio: share(row.hpRaw, row.hpMaxRaw), label: 'Strength', bars }) },
    { key: 'fill', header: 'Equipment fill', label: 'Equipment fill', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', value: row => fill(row.heldRaw, row.neededRaw), renderMissing: true,
      render: row => ratioCell({ ratio: fill(row.heldRaw, row.neededRaw), label: 'Equipment fill', bars }) },
    { key: 'short', header: 'Types short', label: 'Types short', numeric: true, width: 'max-content', priority: 1, value: row => row.typesShort, renderMissing: true,
      text: row => counted(row.typesShort), render: row => row.typesShort ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
  ], [bars]);
  const divisionColumns = useMemo<readonly TableColumn<EquipmentDivision>[]>(() => [
    { key: 'division', header: 'Division', label: 'Division', width: 'minmax(var(--table-name-min), 2fr)', value: divisionName,
      render: row => <span className="division-equipment-name" title={divisionName(row)}><span>{divisionName(row)}</span></span> },
    { key: 'manpower', header: 'Manpower', label: 'Manpower', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 2, value: row => share(row.manpower, row.manpowerRequired), renderMissing: true,
      render: row => ratioCell({ ratio: share(row.manpower, row.manpowerRequired), label: 'Manpower', bars }) },
    { key: 'strength', header: 'Strength', label: 'Strength', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 3, value: row => share(row.hpRaw, row.hpMaxRaw), renderMissing: true,
      render: row => ratioCell({ ratio: share(row.hpRaw, row.hpMaxRaw), label: 'Strength', bars }) },
    { key: 'fill', header: 'Equipment fill', label: 'Equipment fill', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', value: row => row.poolKnown ? fill(row.heldRaw, row.neededRaw) : null, renderMissing: true,
      render: row => ratioCell({ ratio: row.poolKnown ? fill(row.heldRaw, row.neededRaw) : null, label: 'Equipment fill', bars }) },
    { key: 'short', header: 'Types short', label: 'Types short', numeric: true, width: 'max-content', priority: 1, value: row => row.typesShort },
  ], [bars]);
  const poolColumns = useMemo<readonly TableColumn<DivisionPoolType>[]>(() => [
    { key: 'type', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 2fr)', value: typeName, render: row => <span data-tooltip={row.archetype ?? undefined}>{typeName(row)}</span> },
    { key: 'held', header: 'In division', label: 'In division', numeric: true, width: 'max-content', priority: 2, value: row => big(row.heldRaw), renderMissing: true,
      render: row => items(row.heldRaw) ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
    { key: 'needed', header: 'Needed', label: 'Needed', numeric: true, width: 'max-content', priority: 1, value: row => big(row.neededRaw), renderMissing: true,
      render: row => items(row.neededRaw) ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
    { key: 'missing', header: 'Missing', label: 'Missing', numeric: true, width: 'max-content', value: row => big(row.missingRaw), renderMissing: true, render: row => missingCell(row.missingRaw) },
  ], []);
  const typeTip = useCallback((row: EquipmentType) => data ? fitTooltip([typeTooltip(row, data)], viewportHeight) : { title: typeName(row), rows: [] }, [data, viewportHeight]);
  const templateTip = useCallback((row: EquipmentTemplate) => data ? fitTooltip([templateTooltip(row, data)], viewportHeight) : { title: templateName(row), rows: [] }, [data, viewportHeight]);
  const menu = () => data?.poolHour == null || !track ? [] : [jumpItem(runtime, track.id, data.poolHour)];
  const short = data?.divisionsShort ?? null;
  const unknownShort = data?.unknownDivisions === null ? 'Divisions not recorded' : data?.unknownDivisions ? `${usNumber.format(data.unknownDivisions)} ${data.unknownDivisions === 1 ? 'division' : 'divisions'} not recorded` : undefined;
  const header = headerSlot && data && data.missing === null
    ? createPortal(<span className="pill" data-tooltip={unknownShort}>{short === null ? 'Not recorded' : `${usNumber.format(short)} ${short === 1 ? 'division' : 'divisions'} short`}</span>, headerSlot)
    : null;
  let main;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see division equipment." />;
  else if (!gate.open) main = <GateState gate={gate} loading="Loading division equipment…" choose="Choose a country to see its division equipment."
    absent={nameDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`} detail={nameDetail} />;
  else if (state.error) main = <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  else if (!data) main = <WidgetState kind="loading" label="Loading division equipment…" />;
  else if (data.missing !== null) main = <WidgetState kind="unavailable" message="Division equipment wasn’t recorded at this time." detail={data.missing} />;
  else if (data.divisions === 0) main = <WidgetState kind="empty" message={nameDetail ? 'No divisions at this date.' : `${countryName} has no divisions at this date.`} detail={nameDetail} />;
  else main = <>{data.poolHour !== null && <p className="division-equipment-caption">Equipment as of {formatDateTime(data.poolHour)}{unknownShort ? ` · ${unknownShort}` : ''}</p>}
    {view === 'equipment'
      ? <Table rowRevision={row => [row, typeColumns]} key="equipment" label="Division equipment by type" rows={data.types} columns={typeColumns} rowKey={typeKey} sort={MISSING_SORT}
        cellBars={bars} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        onRowClick={row => setDrill({ kind: 'type', key: typeKey(row) })} tooltip={typeTip} menu={menu} />
      : <Table rowRevision={row => [row, templateColumns]} key="template" label="Division equipment by template" rows={data.templates} columns={templateColumns} rowKey={templateKey} sort={FILL_SORT}
        cellBars={bars} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        onRowClick={row => setDrill({ kind: 'template', template: row.template })} tooltip={templateTip} menu={menu} />}</>;
  const type = drill?.kind === 'type' ? data?.types.find(row => typeKey(row) === drill.key) : undefined;
  const drillData = detail.data;
  const drillMenu = () => drillData?.poolHour == null || !track ? [] : [jumpItem(runtime, track.id, drillData.poolHour)];
  const drillHour = (drill && drill.kind !== 'type' ? detail.at ?? detail.hour : state.at ?? state.hour) ?? hour;
  let body = null;
  if (drill && (drill.kind === 'type' ? state.outside : detail.outside)) body = <WidgetState kind="outside" read={drill.kind === 'type' ? state : detail} />;
  else if (drill?.kind === 'type' && state.error) body = <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  else if (drill?.kind === 'type' && !data) body = <WidgetState kind="loading" label="Loading…" read={state} />;
  else if (drill?.kind === 'type') body = type ? <div className="division-equipment-drill">
    <Table rowRevision={row => [row]} label={`${typeName(type)} models`} rows={type.variants} columns={[
      { key: 'model', header: 'Model', label: 'Model', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? 'Design name unavailable',
        render: row => <span data-tooltip={row.variant}>{row.name ?? 'Design name unavailable'}</span> },
      { key: 'held', header: 'In divisions', label: 'In divisions', numeric: true, width: 'max-content', bar: 'visible', value: row => big(row.heldRaw), renderMissing: true,
        render: row => items(row.heldRaw) ?? <span className="division-equipment-missing" data-tooltip="Not recorded">—</span> },
    ]} rowKey={row => row.variant} sort={{ column: 'held', direction: 'descending' }} cellBars={bars} narrow={narrowTableOf(options)} menu={menu} /></div>
    : <WidgetState kind="unavailable" message="This equipment type is not held at this date." />;
  else if (drill && detail.error) body = <WidgetState kind="error" failure={detail.error} onRetry={detail.retry} />;
  else if (drill && !drillData) body = <WidgetState kind="loading" label="Loading…" read={detail} />;
  else if (drill?.kind === 'template' && drillData) body = <div className="division-equipment-drill">
    <Table rowRevision={row => [row, divisionColumns]} label="Divisions of this template" rows={drillData.divisionRows ?? []} columns={divisionColumns} rowKey={divisionKey} sort={FILL_SORT}
      cellBars={bars} narrow={narrowTableOf(options)} onRowClick={row => setDrill({ kind: 'division', division: row.division, template: drill.template })}
      tooltip={row => ({ title: divisionName(row), rows: [
        { label: 'Equipment', value: row.poolKnown ? `${items(row.heldRaw) ?? '—'} of ${items(row.neededRaw) ?? '—'}` : 'Not recorded' },
        { label: 'Types short', value: row.typesShort === null ? 'Not recorded' : usNumber.format(row.typesShort) },
        ...(row.convertingFrom ? [{ label: 'Converting from', value: row.convertingFrom }] : []),
      ] })} menu={drillMenu} /></div>;
  else if (drill?.kind === 'division' && drillData) body = drillData.pool ? <div className="division-equipment-drill">
    <p><b>{divisionName(drillData.pool)}</b> · {drillData.pool.templateName ?? 'Template name unavailable'}</p>
    {!drillData.pool.poolKnown && <WidgetState kind="unavailable" message="This division's equipment was not recorded at this date." />}
    {(drillData.pool.poolKnown || drillData.pool.types.length > 0)
      && <Table rowRevision={row => [row, poolColumns]} label={drillData.pool.poolKnown ? 'Equipment in this division' : 'Equipment this division needs'} rows={drillData.pool.types} columns={poolColumns}
        rowKey={poolKey} sort={MISSING_SORT} cellBars={bars} narrow={narrowTableOf(options)} menu={drillMenu}
        tooltip={row => ({ title: typeName(row), rows: row.variants.map(variant => ({ label: variant.name ?? 'Design name unavailable', value: items(variant.heldRaw) ?? '—' })) })} />}</div>
    : <WidgetState kind="unavailable" message="This division is not in service at this date." />;
  const back = drill?.kind === 'division' && drill.template ? { kind: 'template' as const, template: drill.template } : null;
  const root = useCallback((element: HTMLDivElement | null) => { anchor.current = element; state.root(element); detail.root(element); }, [state.root, detail.root]);
  return <div ref={root} className="division-equipment" data-detail={countryLabel.nameFailure?.message}>{header}
    { }
    {country.chosen && country.error && country.historyHeld ? <WidgetState kind="error" failure={country.error} onRetry={country.retry} /> : null}
    <DrillHost drill={itemDrill} runtime={runtime} subject={drill !== null} mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title={drill?.kind === 'type' ? type ? typeName(type) : 'Equipment' : drill?.kind === 'template'
        ? templateName(drillData?.templates.find(row => row.template === drill.template) ?? data?.templates.find(row => row.template === drill.template) ?? { name: null }) : 'Division'}
      day={drillHour === null ? '' : formatDateTime(drillHour)} canFollow={false} anchor={anchor}
      onClose={() => setDrill(back)} main={main}>{body}</DrillHost></div>;
}
