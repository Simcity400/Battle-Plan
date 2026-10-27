import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { logSettings } from '../../charts/log/logOptions';
import Table, { CellBar, CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { contentTitleHint } from '../../dashboard/contentTitles';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useReachedProduct } from '../../dashboard/reachedProduct';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { CareerRow } from '../../generated/dashboard';
import { careerTooltip, divisionKey as rowKey, divisionName, experienceRatio, medalName, medalsValue, pinnedRecord, validateDivisionCareers } from '../division-service-record/careersData';
import RecordView from '../division-service-record/RecordView';
import { DEFAULT_SORT, OPTIONS } from './definition';
import './mostDecorated.css';

const EMPTY: readonly CareerRow[] = [];
const experienceValue = (row: CareerRow) => row.facts.experienceRaw === null ? null : BigInt(row.facts.experienceRaw);
const killsValue = (row: CareerRow) => row.facts.kills;
const RECORD_LOG = logSettings({});

export default function MostDecoratedWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const playback = useTableHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const options = resolveOptions(OPTIONS, widget.options);
  const selection = useCountriesSelectionAt(widget, runtime, hour, active);
  const tags = selection.countries === undefined ? undefined : selection.countries === null ? null : [...selection.countries].sort();
  const request = track && window && tags !== undefined && !selection.selectionReason ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'division-careers', params: { branchId: track.branch.id, hour, tags } } : null;
  const state = useReachedProduct(runtime.client, track?.source, request, JSON.stringify([track?.branch.id, tags]), active, validateDivisionCareers);
  const data = state.data;
  const rows = data?.divisions ?? EMPTY;
  const templates = useMemo(() => new Map((data?.templates ?? []).map(template => [template.template, template])), [data]);
  const [visible, setVisible] = useState<readonly CareerRow[]>(EMPTY);
  const wanted = useMemo(() => data ? visible.flatMap(row => row.tag ? [{ hour: data.hour, tag: row.tag }] : []) : [], [visible, data]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const viewportHeight = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  useDiagnostics(onDiagnostics, drill.breakdown === null ? data?.warnings ?? null : null);
  useEffect(() => { dispatch({ type: 'close' }); }, [track?.source.id, track?.source.revision, track?.branch.id]);
  const open = useCallback((row: CareerRow) => {
    if (!data) return;
    dispatch({ type: 'open', breakdown: row.division, hour: data.hour,
      playing: widget.time.mode === 'follow' && runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  }, [data, widget.time.mode, runtime.clock, hour]);
  const country = useCallback((row: CareerRow) => row.tag && data ? labelAt(data.hour)(row.tag) : { name: 'Country not recorded' }, [labelAt, data]);
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
      render: row => row.facts.kills === null ? <span className="decorated-missing" data-tooltip="Not recorded">—</span> : row.facts.kills.toLocaleString('en-US') },
  ], [templates, country, data, options.cellBars]);
  const tooltip = useCallback((row: CareerRow) => data ? fitTooltip([careerTooltip(row, data, country(row).name, templates.get(row.template)?.name ?? null)], viewportHeight)
    : { title: divisionName(row.name), rows: [] }, [data, country, templates, viewportHeight]);
  let main;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see divisions." />;
  else if (!window) main = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (selection.group.error) main = <WidgetState kind="error" detail={selection.group.error} onRetry={selection.group.retry} />;
  else if (selection.selectionReason) main = <WidgetState kind="unavailable" message={selection.selectionReason} />;
  else if (state.error) main = <WidgetState kind="error" detail={state.error} onRetry={state.retry} />;
  else if (!data) main = <WidgetState kind="loading" label="Loading divisions…" />;
  else if (data.missing) main = <WidgetState kind="unavailable" message="Divisions weren’t recorded at this time." />;
  else if (!rows.length) main = selection.known ? <WidgetState kind="empty" message="No divisions at this date." /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  else main = <Table label="Most decorated divisions" rows={rows} columns={columns} rowKey={rowKey} rank rowHeight={40}
    sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} onRowClick={open} tooltip={tooltip}
    onVisibleRows={setVisible} hoursPerSecond={hoursPerSecond} menu={row => [{ label: 'Open service record', run: () => open(row) }]} />;
  const pickedHour = drillHour(drill, Math.floor(runtime.clock.getHour()) + (track?.axisOffset ?? 0)) ?? hour;
  return <div ref={anchor} className="most-decorated">
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null} title="Division service record"
      day={formatDateTime(pickedHour)} following={drill.follow} canFollow anchor={anchor}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => { if (drill.breakdown) dispatch({ type: 'follow' }); }}
      onPin={drill.breakdown && track ? () => runtime.addWidgetBelow(widget.id, pinnedRecord(widget, drill.breakdown!, track, pickedHour, drill.follow)) : undefined}
      main={main}>
      {drill.breakdown && track && <RecordView runtime={runtime} trackId={track.id} division={drill.breakdown} hour={pickedHour} show="both"
        log={RECORD_LOG} live={drill.follow} active={active} onDiagnostics={onDiagnostics} />}
    </DrillHost></div>;
}
