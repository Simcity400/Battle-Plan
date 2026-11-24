import { nameReadText } from '../../dashboard/nameReads.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
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
import { useChoiceHour } from '../../dashboard/atHead';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useGate } from '../../dashboard/regionGate';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { countdownTexts, electionValues, electionsAt, electionsState, electionsStructure, electionTooltip, electionWarnings, termAt, termSortValues, termText, validateElectionsReply } from './nextElectionsData';
import type { ElectionRow } from './nextElectionsData';
import './nextElections.css';

const NO_ROWS: readonly ElectionRow[] = [];
const NO_WARNINGS: string[] = [];
const ALL_PARAMS = {} as const;
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
  const directSelection = widget.countries.kind === 'tags' || widget.countries.kind === 'selected';
  const selected = selectedCountryKeys(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  const fallback = useMemo(() => selected, [selectedKey]);
  const params = useMemo(() => {
    if (!track) return null;
    if (directSelection && fallback.length > 0) return { countries: fallback.map(countryFromKey) };
    return ALL_PARAMS;
  }, [track, directSelection, fallback]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'next-elections', params, { active, validate: validateElectionsReply, structure: electionsStructure });
  const data = head.data;
  const hour = head.at ?? track?.branch.lastHour ?? 0;
  const needsGroups = widget.countries.kind === 'group';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const match = useMemo(() => {
    if (directSelection) {
      return { ready: true, matches: () => true, known: () => true, reason: fallback.length ? null : 'Choose a country in the widget settings.' };
    }
    return countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour);
  }, [directSelection, widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const choiceAt = useChoiceHour(runtime.clock, active, () => head.atNow() ?? hour, () => head.shownAt(), countryMatchChangeHours(match.matches), head.live.subscribe);
  const view = useMemo(() => data && data.recorded && match.ready && !match.reason ? electionsAt(data, choiceAt, match.matches) : null,
    [data, choiceAt, match]);
  const rows = view?.rows ?? NO_ROWS;
  const trackHour = useCallback((clockHour: number) => {
    const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
    return Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  }, [widget, track, runtime.tracks]);
  const shown = useCallback(() => head.shownAt() ?? trackHour(runtime.clock.getHour()), [head.shownAt, runtime.clock, trackHour]);
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

  let body;
  if (!track) body = <WidgetState kind="empty" icon="ballot" message="Load a campaign to see elections." />;
  else if (head.read === 'off') body = <WidgetState kind="outside" />;
  else if (head.error && !data) body = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (needsGroups && groups.error && !groups.data) body = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (!data || !match.ready) body = <WidgetState kind="loading" label="Loading elections…" />;
  else if (!data.recorded) body = <WidgetState kind="unavailable" message="Election schedules were not recorded at this date." />;
  else {
    const state = view && electionsState(view, membershipUnrecordedAt(widget.countries, groups.data, choiceAt));
    if (state) body = state.kind === 'empty' ? <WidgetState kind="empty" icon="ballot" message={state.message} /> : <WidgetState kind="unavailable" message={state.message} />;
    else body = <Table rowRevision={row => [row, columns]} key={`${track.source.id}:${track.branch.id}`} label="Next elections" rows={rows} columns={columns} rowKey={rowKey} tieKey={tieKey}
      sort={sort} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)}
      menu={menu} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} live={sortLive} />;
  }
  return <div className="next-elections" ref={head.root}>
    {needsGroups && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {body}
  </div>;
}
