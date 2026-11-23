import { nameReadText } from '../../dashboard/nameReads.ts';
import { recordedHour } from '../../dashboard/regionTime.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import LiveText, { liveTexts } from '../../charts/LiveText';
import type { LiveTexts, TextClock } from '../../charts/LiveText';
import { liveAt } from '../../charts/live';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { meterValue } from '../../charts/meter/meterValue';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { dayLabel } from '../../charts/draw';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useAtHead } from '../../dashboard/atHead';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { POLITICS_HISTORY, readAt } from '../../dashboard/politicsHistory';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { changeHours, countdownTexts, electionValues, electionsAt, electionsState, electionTooltip, electionWarnings, settledHour, termAt, termSortValues, termText, validateElectionsReply } from './nextElectionsData';
import type { ElectionRow } from './nextElectionsData';
import './nextElections.css';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

const NO_ROWS: readonly ElectionRow[] = [];
const NO_WARNINGS: string[] = [];
const NO_HOURS: readonly number[] = [];
const rowKey = (row: ElectionRow) => row.key;
const tieKey = (row: ElectionRow) => row.tag;

function TermCell({ row, live, clock, shown, cellBars }: { row: ElectionRow; live: LiveTexts; clock: TextClock; shown: () => number; cellBars: boolean }) {
  const cell = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    if (!cellBars) return;
    const paint = () => {
      const meter = cell.current?.querySelector<HTMLElement>('.progress-meter'), fill = meter?.querySelector('i');
      const ratio = meterValue(termAt(row, shown()));
      if (!meter || !fill || ratio === null || fill.style.width === `${ratio * 100}%`) return;
      fill.style.width = `${ratio * 100}%`;
      meter.setAttribute('aria-valuenow', String(ratio));
    };
    paint();
    const stops = [clock.subscribeFrame(paint), clock.subscribe(paint)];
    return () => stops.forEach(stop => stop());
  }, [row, clock, shown, cellBars]);
  return <span ref={cell} className="election-term"><LiveText live={live} at={`term\u0000${row.key}`} />
    {cellBars && <ProgressMeter value={termAt(row, shown())} state="live" label="Term passed" />}</span>;
}

export function electionColumns(sortName: (tag: string) => string, label: CountryLabel, cellBars: boolean, live: LiveTexts, clock: TextClock,
  shown: () => number): TableColumn<ElectionRow>[] {
  const at = (column: 'term' | 'days' | 'next') => (row: ElectionRow) => electionValues(shown())[column](row);
  return [
    { key: 'country', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 2fr)',
      value: row => sortName(row.tag), text: row => label(row.tag).name, render: row => <CountryCell {...label(row.tag)} /> },
    { key: 'term', header: 'Term', label: 'Term', numeric: true, renderMissing: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 1,
      value: at('term'), sortsLive: true, text: row => { const term = termAt(row, shown()); return term === null ? null : termText(term); },
      render: row => <TermCell row={row} live={live} clock={clock} shown={shown} cellBars={cellBars} /> },
    { key: 'days', header: 'Days left', label: 'Days left', numeric: true, width: 'max-content', priority: 2, inTooltip: true,
      value: at('days'), render: row => <LiveText live={live} at={`days\u0000${row.key}`} /> },
    { key: 'next', header: 'Next election', label: 'Next election', numeric: true, width: 'max-content', priority: 3, inTooltip: true,
      value: at('next'), render: row => dayLabel(row.next!) },
  ];
}

export default function NextElectionsWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: POLITICS_HISTORY,
    params: { branchId: track.branch.id, mode: 'elections' } }) : null;
  const { data, error, retry } = useProduct<PoliticsHistoryReply>(runtime.client, request, active, validateElectionsReply, track?.source);
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedCountryKeys(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const hours = useMemo(() => data ? changeHours(data, countryMatchChangeHours(match.matches)) : NO_HOURS, [data, match]);
  const trackHour = useCallback((clockHour: number) => {
    const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
    return Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  }, [widget, track, runtime.tracks]);
  const following = widget.time.mode === 'follow';
  const hour = useAtHead(runtime.clock, active && following, clockHour => settledHour(hours, trackHour(clockHour)));
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const inside = !!track && !!window && recordedHour(track, hour) !== null;
  const read = !!data && readAt(data.known, hour);
  const view = useMemo(() => data && read && match.ready && !match.reason ? electionsAt(data, hour, match.matches) : null,
    [data, read, hour, match]);
  const rows = view?.rows ?? NO_ROWS;
  const shown = useCallback(() => trackHour(runtime.clock.getHour()), [runtime.clock, trackHour]);
  const live = useMemo(() => liveTexts(runtime.clock, shown, at => countdownTexts(rows, at)), [runtime.clock, shown, rows]);
  const sortLive = useMemo(() => liveAt(runtime.clock, () => shown(), termSortValues(rows)), [runtime.clock, shown, rows]);
  const [visible, setVisible] = useState<readonly ElectionRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((rows: readonly ElectionRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => visible.map(row => ({ hour, tag: row.tag })), [visible, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const sortingNames = useMemo(() => sort.column === 'country' ? rows.map(row => ({ hour, tag: row.tag })) : [], [rows, sort.column, hour]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const columns = useMemo(() => electionColumns(tag => nameReadText(names.get(nameKey(hour, tag))), labelAt(hour), options.cellBars === true, live,
    runtime.clock, shown), [names, labelAt, hour, options.cellBars, live, runtime.clock, shown]);
  const tooltip = useCallback((row: ElectionRow) => fitTooltip([electionTooltip(row, labelAt(hour)(row.tag), shown())], viewport), [labelAt, hour, viewport, shown]);
  const menu = useCallback((row: ElectionRow) => {
    if (!track || row.next === null) return [];
    const item = jumpItem(runtime, track.id, row.next);
    return item.disabled ? [] : [item];
  }, [runtime, track]);
  const warnings = useMemo(() => data ? electionWarnings(data, view) : NO_WARNINGS, [data, view]);
  useEffect(() => {
    onDiagnostics(warnings);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="ballot" message="Load a campaign to see elections." />;
  if (!window || !inside) return <WidgetState kind="outside" />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading elections…" /></>;
  if (!read) return <>{groupFailure}<WidgetState kind="unavailable" message="Election schedules were not recorded at this date." /></>;
  const state = view && electionsState(view, membershipUnrecordedAt(widget.countries, groups.data, hour));
  if (state) return <>{groupFailure}{state.kind === 'empty' ? <WidgetState kind="empty" icon="ballot" message={state.message} /> : <WidgetState kind="unavailable" message={state.message} />}</>;
  return <div className="next-elections">{groupFailure}
    <Table rowRevision={row => [row, columns]} key={`${track.source.id}:${track.branch.id}`} label="Next elections" rows={rows} columns={columns} rowKey={rowKey} tieKey={tieKey}
      sort={sort} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)}
      menu={menu} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} live={sortLive} />
  </div>;
}
