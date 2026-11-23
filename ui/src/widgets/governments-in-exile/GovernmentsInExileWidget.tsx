import { nameReadText } from '../../dashboard/nameReads.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { fitTooltip } from '../../charts/tooltip';
import { usNumber } from '../../charts/numberFormat';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from '../../dashboard/groups';
import { useChoiceHour } from '../../dashboard/atHead';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { exileNumbers, exilesAt, exileStructure, exileTableValues, exileTooltip, exileWarnings, legitimacyText, manpowerText, validateExiles } from './governmentsInExileData';
import { mapLive } from '../../charts/live';
import type { LiveValues } from '../../charts/live';
import type { ExileRow } from './governmentsInExileData';
import './governmentsInExile.css';

const NO_ROWS: readonly ExileRow[] = [];
const NO_WARNINGS: string[] = [];
const PARAMS = { view: 'exiles' } as const;
const rowKey = (row: ExileRow) => row.key;
const tieKey = (row: ExileRow) => row.tag;
const rowOf = (key: string) => key.slice(0, key.lastIndexOf('\u0000'));
const count = (value: unknown) => usNumber.format(value as number);

export function exileColumns(sortName: (tag: string) => string, label: CountryLabel, values: () => LiveValues<number>): TableColumn<ExileRow>[] {
  return [
    { key: 'exile', header: 'Exile', label: 'Exile', width: 'minmax(var(--table-name-min), 2fr)',
      value: row => sortName(row.tag), text: row => label(row.tag).name, render: row => <CountryCell {...label(row.tag)} /> },
    { key: 'host', header: 'Host', label: 'Host', width: 'minmax(var(--table-name-min), 2fr)', renderMissing: true, priority: 1, inTooltip: true,
      value: row => row.host === null ? null : sortName(row.host), text: row => row.host === null ? null : label(row.host).name,
      render: row => row.host === null ? 'Not recorded' : <CountryCell {...label(row.host)} /> },
    { key: 'legitimacy', header: 'Legitimacy', label: 'Legitimacy', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 2, inTooltip: true,
      value: () => null, paint: value => legitimacyText(value as number), bar: 100 },
    { key: 'manpower', header: 'Manpower', label: 'Manpower', numeric: true, width: 'max-content', priority: 3, inTooltip: true,
      value: () => null, paint: (_, key) => manpowerText(exileNumbers(values(), rowOf(key))) },
    { key: 'air', header: 'Air crews', label: 'Air crews', numeric: true, width: 'max-content', priority: 5, inTooltip: true, value: () => null, paint: count },
    { key: 'leaders', header: 'Army leaders', label: 'Army leaders', numeric: true, width: 'max-content', priority: 4, inTooltip: true, value: () => null, paint: count },
  ];
}

export default function GovernmentsInExileWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'politics-state', track ? PARAMS : null, { active, validate: validateExiles, structure: exileStructure });
  const data = useMemo(() => head.data && head.at !== null ? { ...head.data, hour: head.at } : undefined, [head.data, head.at]);
  const live = head.live;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const tableLive = useMemo(() => mapLive(reporting(live, () => shown(shownAt.current())), exileTableValues), [live, shown]);
  const hour = data?.hour ?? track?.branch.lastHour ?? 0;
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedCountryKeys(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const unrecorded = useCallback((at: number) => membershipUnrecordedAt(widget.countries, groups.data, at), [widget.countries, groups.data]);
  const choiceAt = useChoiceHour(runtime.clock, active, () => head.atNow() ?? hour, () => head.shownAt(), countryMatchChangeHours(match.matches), head.live.subscribe);
  const view = useMemo(() => data && match.ready && !match.reason ? exilesAt({ exiles: data.exiles, hour: choiceAt }, match.matches, unrecorded) : null,
    [data, match, unrecorded, choiceAt]);
  const rows = view?.rows ?? NO_ROWS;
  const [visible, setVisible] = useState<readonly ExileRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((rows: readonly ExileRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => visible.flatMap(row => [{ hour, tag: row.tag }, ...row.host === null ? [] : [{ hour, tag: row.host }]]), [visible, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const sortingNames = useMemo(() => sort.column === 'exile' ? rows.map(row => ({ hour, tag: row.tag }))
    : sort.column === 'host' ? rows.flatMap(row => row.host === null ? [] : [{ hour, tag: row.host }]) : [], [rows, sort.column, hour]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const columns = useMemo(() => exileColumns(tag => nameReadText(names.get(nameKey(hour, tag))), labelAt(hour), live.read), [names, labelAt, hour, live]);
  const tooltip = useCallback((row: ExileRow) => fitTooltip([exileTooltip(exileNumbers(live.read(), row.key), hour, labelAt(hour)(row.tag),
    row.host === null ? null : labelAt(hour)(row.host))], viewport), [labelAt, hour, viewport, live]);
  const warnings = useMemo(() => data ? exileWarnings(data, view) : NO_WARNINGS, [data, view]);
  useEffect(() => {
    onDiagnostics(warnings);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, onDiagnostics]);

  let body;
  if (!track) body = <WidgetState kind="empty" icon="flag" message="Load a campaign to see governments in exile." />;
  else if (head.read === 'off') body = <WidgetState kind="outside" />;
  else if (head.error && !data) body = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (needsGroups && groups.error && !groups.data) body = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (!data || !match.ready) body = <WidgetState kind="loading" label="Loading governments in exile…" />;
  else if (data.unavailable.some(entry => entry.label === 'exile status')) body = <WidgetState kind="unavailable" message="Exiles were not recorded at this date." />;
  else if (!rows.length && view?.membership) body = <WidgetState kind="unavailable" message={view.membership} />;
  else if (!rows.length) body = <WidgetState kind="empty" icon="flag" message="No governments in exile at this date." />;
  else body = <Table rowRevision={row => [row, columns]} key={`${track.source.id}:${track.branch.id}`} label="Governments in exile" rows={rows} columns={columns} rowKey={rowKey} tieKey={tieKey}
    sort={sort} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)}
    tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} live={tableLive} />;
  return <div className="governments-in-exile" ref={head.root}>
    {headerSlot && view && createPortal(<span className="pill exile-header-count">{usNumber.format(rows.length)} in exile</span>, headerSlot)}
    {needsGroups && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {body}
  </div>;
}
