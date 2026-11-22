import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
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
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { usePlaybackHour } from '../../dashboard/follow/useFollow';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { careerTooltip, careersStructure, divisionKey as rowKey, divisionName, experienceRatio, medalName, medalsValue, pinnedRecord, validateDivisionCareers } from '../division-service-record/careersData';
import RecordView from '../division-service-record/RecordView';
import { DEFAULT_SORT, OPTIONS } from './definition';
import './mostDecorated.css';
import type { CareerRow } from '../../generated/dashboard';

const EMPTY: readonly CareerRow[] = [];
const experienceValue = (row: CareerRow) => row.facts.experienceRaw === null ? null : BigInt(row.facts.experienceRaw);
const killsValue = (row: CareerRow) => row.facts.kills;
const RECORD_LOG = logSettings({});

export default function MostDecoratedWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const playback = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const options = resolveOptions(OPTIONS, widget.options);
  const selection = useCountriesSelectionAt(widget, runtime, hour, active);
  const tags = selection.countries === undefined ? undefined : selection.countries === null ? null : [...selection.countries].sort();
  const state = useHeadProduct(runtime, widget, 'division-careers', track && window && tags !== undefined && !selection.selectionReason ? { countries: tags?.map(countryFromKey) ?? null } : null,
    { active, validate: validateDivisionCareers, structure: careersStructure });
  useShownEachFrame(runtime.clock, state.shownAt, state.live);
  const data = state.data;
  const dataHour = state.hour ?? hour;
  const rows = data?.divisions ?? EMPTY;
  const templates = useMemo(() => new Map((data?.templates ?? []).map(template => [template.template, template])), [data]);
  const [visible, setVisible] = useState<readonly CareerRow[]>(EMPTY);
  const wanted = useMemo(() => data ? visible.flatMap(row => row.tag ? [{ hour: dataHour, tag: row.tag }] : []) : [], [visible, data, dataHour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const viewportHeight = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  useDiagnostics(onDiagnostics, drill.breakdown === null ? data?.warnings ?? null : null);
  useEffect(() => { dispatch({ type: 'close' }); }, [track?.source.id, track?.source.revision, track?.branch.id]);
  const open = useCallback((row: CareerRow) => {
    if (!data) return;
    dispatch({ type: 'open', breakdown: row.division, hour: dataHour,
      playing: widget.time.mode === 'follow' && runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  }, [data, dataHour, widget.time.mode, runtime.clock, hour]);
  const country = useCallback((row: CareerRow) => row.tag && data ? labelAt(dataHour)(row.tag) : { name: 'Country not recorded' }, [labelAt, data, dataHour]);
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
  const tooltip = useCallback((row: CareerRow) => data ? fitTooltip([{ ...careerTooltip(row, data, dataHour, country(row).name, templates.get(row.template)?.name ?? null), ...countryNameDetails(country(row)) }], viewportHeight)
    : { title: divisionName(row.name), rows: [] }, [data, country, templates, viewportHeight]);
  let main;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see divisions." />;
  else if (!window) main = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
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
  const pickedHour = drillHour(drill, Math.floor(playback) + (track?.axisOffset ?? 0)) ?? hour;
  const [answered, setAnswered] = useState<number | undefined>();
  const shownHour = drill.breakdown ? answered ?? pickedHour : pickedHour;
  const root = useCallback((element: HTMLDivElement | null) => { anchor.current = element; state.root(element); }, [state.root]);
  return <div ref={root} className="most-decorated">
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null} title="Division service record"
      day={formatDateTime(shownHour)} following={drill.follow} canFollow anchor={anchor}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => { if (drill.breakdown) dispatch({ type: 'follow' }); }}
      onPin={drill.breakdown && track ? () => runtime.addWidgetBelow(widget.id, pinnedRecord(widget, drill.breakdown!, track, shownHour, drill.follow)) : undefined}
      main={main}>
      {drill.breakdown && track && <RecordView runtime={runtime} widget={widget} trackId={track.id} division={drill.breakdown} hour={pickedHour} show="both"
        log={RECORD_LOG} live={drill.follow} active={active} onDiagnostics={onDiagnostics} onAnswered={setAnswered} />}
    </DrillHost></div>;
}
