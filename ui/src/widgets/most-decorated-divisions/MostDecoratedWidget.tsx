import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo,  useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { logSettings } from '../../charts/log/logOptions';
import Table, { CellBar, CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { contentTitleHint } from '../../dashboard/contentTitles';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { careerTooltip, careersStructure, divisionKey as rowKey, divisionName, experienceRatio, medalName, medalsValue, pinnedRecord, validateDivisionCareers } from '../division-service-record/careersData';
import RecordView from '../division-service-record/RecordView';
import { DEFAULT_SORT, OPTIONS } from './definition';
import './mostDecorated.css';
import type { CareerRow } from '../../generated/dashboard';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';

const EMPTY: readonly CareerRow[] = [];
const experienceValue = (row: CareerRow) => row.facts.experienceRaw === null ? null : BigInt(row.facts.experienceRaw);
const killsValue = (row: CareerRow) => row.facts.casualtiesTaken;
const RECORD_LOG = logSettings({});

export default function MostDecoratedWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const selection = useCountriesSelectionAt(widget, runtime, active);
  const hour = selection.hour;
  const drillTime = drillControl.region.action;
  const drillRegion = useRegion(widget, drillTime);
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const tags = selection.countries === undefined ? undefined : selection.countries === null ? null : [...selection.countries].sort();
  const state = useHeadProduct(runtime, selection.gate, 'division-careers', track && tags !== undefined && !selection.selectionReason ? { countries: tags?.map(countryFromKey) ?? null } : null,
    { active, validate: validateDivisionCareers, structure: careersStructure });
  useShownEachFrame(runtime.clock, state.shownAt, state.live);
  const data = state.data;
  const dataHour = state.at;
  const rows = data?.divisions ?? EMPTY;
  const templates = useMemo(() => new Map((data?.templates ?? []).map(template => [template.template, template])), [data]);
  const [visible, setVisible] = useState<readonly CareerRow[]>(EMPTY);
  const wanted = useMemo(() => data && dataHour !== null ? visible.flatMap(row => row.tag ? [{ hour: dataHour, tag: row.tag }] : []) : [], [visible, data, dataHour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const viewportHeight = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  useDiagnostics(onDiagnostics, drill.breakdown === null ? data?.warnings ?? null : null);
  useEffect(() => { dispatch({ type: 'close' }); }, [track?.source.id, track?.source.revision, track?.branch.id]);
  const open = useCallback((row: CareerRow) => {
    if (!data || dataHour === null) return;
    drillControl.show(row.division, dataHour);
  }, [data, dataHour, widget.time.mode, runtime.clock, hour]);
  const country = useCallback((row: CareerRow) => row.tag && data && dataHour !== null ? labelAt(dataHour)(row.tag) : { name: 'Country not recorded' }, [labelAt, data, dataHour]);
  const columns = useMemo<readonly TableColumn<CareerRow>[]>(() => [
    { key: 'division', header: 'Division', label: 'Division', sortable: false, width: 'minmax(var(--table-name-min), 2fr)', value: row => divisionName(row.name),
      render: row => { const template = templates.get(row.template); const named = country(row);
        return <span className="decorated-division" data-tooltip={row.division}>{template?.icon ? <img src={template.icon} alt="" /> : <ArtFallback icon="helmet" />}
          <span className="decorated-division-text"><span>{divisionName(row.name)}</span>
            <CountryCell {...named} /></span></span>; } },
    { key: 'medals', header: 'Medals', label: 'Medals', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1.5fr)', align: 'start', value: medalsValue, renderMissing: true,
      text: row => row.facts.medals === null ? 'Not recorded' : String(row.facts.medals),
      render: row => row.facts.medals === null ? <span className="decorated-missing" data-tooltip={row.facts.historyMissing ?? 'Not recorded'}>—</span>
        : <span className="decorated-medals"><b>{row.facts.medals}</b>{data && row.facts.awards.map(award =>
          <span key={award.medal} className="decorated-medal-chip" data-tooltip={contentTitleHint(data.medals[award.medal]?.key)}>{medalName(data.medals[award.medal])}{award.count > 1 ? ` ×${award.count}` : ''}</span>)}</span> },
    { key: 'experience', header: 'Experience', label: 'Experience', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 2, value: experienceValue, renderMissing: true,
      text: row => row.facts.levelName ?? 'Not recorded',
      render: row => { const ratio = experienceRatio(row.facts);
        return <CellBar value={options.cellBars === true && ratio !== null ? ratio : null} maximum={1}>{row.facts.levelName ?? '—'}</CellBar>; } },
    { key: 'kills', header: 'Enemy soldiers killed', label: 'Enemy soldiers killed', numeric: true, width: 'max-content', priority: 1, value: killsValue, renderMissing: true,
      render: row => row.facts.casualtiesTaken === null ? <span className="decorated-missing" data-tooltip="Not recorded">—</span> : row.facts.casualtiesTaken.toLocaleString('en-US') },
  ], [templates, country, data, options.cellBars]);
  const tooltip = useCallback((row: CareerRow) => data && dataHour !== null ? fitTooltip([{ ...careerTooltip(row, data, dataHour, country(row).name, templates.get(row.template)?.name ?? null), ...countryNameDetails(country(row)) }], viewportHeight)
    : { title: divisionName(row.name), rows: [] }, [data, country, templates, viewportHeight]);
  let main;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see divisions." />;
  else if (state.outside || selection.outside) main = <WidgetState kind="outside" read={state} />;
  else if (selection.group.error) main = <WidgetState kind="error" failure={selection.group.error} onRetry={selection.group.retry} />;
  else if (selection.selectionReason) main = <WidgetState kind="unavailable" message={selection.selectionReason} />;
  else if (state.error) main = <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  else if (!data) main = <WidgetState kind="loading" label="Loading divisions…" />;
  else if (data.missing) main = <WidgetState kind="unavailable" message="Divisions weren’t recorded at this time." />;
  else if (!rows.length) main = <WidgetState kind="empty" message="No divisions at this date." />;
  else main = <Table rowRevision={row => [row, columns]} label="Most decorated divisions" rows={rows} columns={columns} rowKey={rowKey} rank rowHeight={40}
    sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} onRowClick={open} tooltip={tooltip}
    onVisibleRows={setVisible} hoursPerSecond={hoursPerSecond} menu={row => [{ label: 'Open service record', run: () => open(row) }]} />;
  const [answered, setAnswered] = useState<number | undefined>();
  const shownHour = drill.breakdown ? answered ?? drillAt : drillAt;
  const root = useCallback((element: HTMLDivElement | null) => { anchor.current = element; state.root(element); }, [state.root]);
  return <div ref={root} className="most-decorated">
    <DrillHost drill={drillControl} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} title="Division service record"
      day={shownHour === null ? '' : formatDateTime(shownHour)} canFollow anchor={anchor}
      onClose={() => dispatch({ type: 'close' })}
      pinRefused={drillControl.pinRefused} onPin={drill.breakdown && track && drillControl.time() ? () => {
        const time = drillControl.time();
        const history = time?.value.mode === 'pin' ? drillControl.gate.span(track.branch.firstHour, drillControl.hour ?? Number.NaN) : null;
        const kept = history ? drillControl.time(history) : time;
        if (kept) runtime.addWidgetBelow(widget.id, pinnedRecord(widget, drill.breakdown!, track, kept));
      } : undefined}
      main={main}>
      {drill.breakdown && track && <RecordView runtime={runtime} widget={widget} trackId={track.id} division={drill.breakdown} time={drillTime} show="both"
        log={RECORD_LOG} active={active} onDiagnostics={onDiagnostics} onAnswered={setAnswered} />}
    </DrillHost></div>;
}
