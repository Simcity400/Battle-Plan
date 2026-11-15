import { nameReadText } from '../../dashboard/nameReads.ts';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles.ts';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { jumpItem } from '../../charts/jump';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from '../../dashboard/groups';
import { useChoiceHour } from '../../dashboard/atHead';
import { useHeadProduct } from '../../dashboard/headProduct';
import { usePushedLooksHour } from '../../dashboard/pushedLooks';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useShownHour, useWhyCommit } from '../../dashboard/widgetPerf.ts';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { STRENGTH, garrisonBar, garrisonCell, garrisonText, garrisonTone, occupationTooltip, occupiedRows, percentCell, stateName, validateOccupiedStates } from './occupiedStatesData.ts';
import type { OccupiedRow } from './occupiedStatesData';
import './occupiedStates.css';
import { useStructureState } from '../../dashboard/structurePace.ts';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly OccupiedRow[] = [];
const PARAMS = {} as const;
const rowKey = (row: OccupiedRow) => row.key;
const painted = (value: unknown) => percentCell(BigInt(Math.trunc(value as number)).toString());
/** The countries the rows name, each once, sorted, as one text. */
const countriesOf = (rows: readonly OccupiedRow[]) => [...new Set(rows.flatMap(row => [row.value.owner, row.value.occupier, row.value.occupied]
  .filter((tag): tag is string => tag !== null)))].sort().join(',');

/** Every occupied state at the playback date: the hour's rows followed at the head as a pushed panel (names, countries,
 * garrisons and laws), every number painted from its live cell: resistance, compliance, the garrison's amount, and the
 * garrison's strength as its bar and red. */
export default function OccupiedStatesWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const viewportHeight = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const role = options.countryRole === 'occupied' ? 'occupied' : 'occupier';
  const head = useHeadProduct(runtime, widget, 'occupied-states', track ? PARAMS : null, { active, validate: validateOccupiedStates });
  const data = head.data;
  const live = head.live;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const tableLive = useMemo(() => reporting(live, () => shown(shownAt.current())), [live, shown]);
  // With no table drawn (no occupied state, or a state saying why), the panel's own hour is what the widget shows.
  useEffect(() => { if (head.data) shown(shownAt.current()); });
  const hour = head.at ?? track?.branch.lastHour ?? 0;
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedTags(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  // A held panel's rows are chosen again where the chosen group changes, at the hour its values are current through.
  const choiceAt = useChoiceHour(runtime.clock, active, () => head.atNow() ?? hour, () => head.shownAt(), countryMatchChangeHours(match.matches), head.live.subscribe);
  const view = useMemo(() => data && match.ready && !match.reason
    ? occupiedRows(data, widget.countries.kind === 'all' ? null : tag => match.matches(tag, choiceAt), role, membershipUnrecordedAt(widget.countries, groups.data, choiceAt)) : null,
  [data, match, widget.countries, groups.data, choiceAt, role]);
  const rows = view?.rows ?? NO_ROWS;
  // The countries of the rows drawn, for their names and flags: news only when other countries are drawn (a push or a
  // re-sort moves rows, not countries), and paced with the widget's other structure, so at most a commit a slot.
  const [drawn, setDrawn] = useStructureState(runtime.clock, '');
  const onVisibleRows = useCallback((shownRows: readonly OccupiedRow[]) => setDrawn(countriesOf(shownRows)), [setDrawn]);
  const tagsKey = useMemo(() => countriesOf(rows), [rows]);
  const stableTags = useMemo(() => tagsKey ? tagsKey.split(',') : [], [tagsKey]);
  // Pushed, each panel comes at a new hour: the countries are named where one of their identities last changed, so a
  // push asks no name and commits once.
  const namesHour = usePushedLooksHour(runtime, widget, track, stableTags, head.read === 'followed', hour).hour;
  const wanted = useMemo(() => drawn ? drawn.split(',').map(tag => ({ hour: namesHour, tag })) : [], [drawn, namesHour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  // A name sort needs every name; flags still load only for mounted rows.
  const sortingNames = useMemo(() => sort.column === 'owner' || sort.column === 'occupier'
    ? rows.flatMap(row => row.value[sort.column as 'owner' | 'occupier'] === null ? [] : [{ hour: namesHour, tag: row.value[sort.column as 'owner' | 'occupier']! }]) : [],
  [rows, sort.column, namesHour]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const laws = useMemo(() => new Map((data?.laws ?? []).map(law => [law.token, law])), [data]);
  const redRaw = data?.garrisonRedRaw ?? null;
  const columns = useMemo<readonly TableColumn<OccupiedRow>[]>(() => {
    const label = labelAt(namesHour);
    const country = (field: 'owner' | 'occupier', title: string, priority?: number): TableColumn<OccupiedRow> => ({ key: field, header: title, label: title, renderMissing: true,
      width: 'minmax(150px, 1fr)', priority,
      value: row => { const tag = row.value[field]; return tag === null ? null : nameReadText(names.get(nameKey(namesHour, tag))); },
      text: row => { const tag = row.value[field]; return tag === null ? null : label(tag).name; },
      render: row => { const tag = row.value[field]; return tag === null ? 'Unknown' : <CountryCell {...label(tag)} />; } });
    const lawName = (row: OccupiedRow) => row.value.law === null ? row.value.lawMissing ?? 'Not recorded'
      : contentTitle(laws.get(row.value.law)?.name, row.value.law, 'Occupation law name unavailable');
    return [
      { key: 'state', header: 'State', label: 'State', renderMissing: true, width: 'minmax(160px, 1.2fr)', value: row => row.value.name ?? null, render: stateName },
      country('owner', 'Owner', 1), country('occupier', 'Occupier'),
      { key: 'resistance', header: 'Resistance', label: 'Resistance', numeric: true, bar: 10000000, width: '150px', value: () => null, paint: painted },
      { key: 'compliance', header: 'Compliance', label: 'Compliance', numeric: true, bar: 10000000, width: '150px', priority: 2, value: () => null, paint: painted },
      // The required amount is painted from its cell, the strength bar and its red from the strength's cell; an unmet
      // requirement's empty bar and red are the row's structure.
      { key: 'garrison', header: 'Garrison', label: 'Garrison', numeric: true, width: '175px', priority: 3, bar: 100000n, value: () => null,
        paint: value => garrisonText(value as number), missingText: () => '-',
        barOf: row => garrisonCell(row.value).bar, barCell: { id: STRENGTH, value: garrisonBar, tone: garrisonTone(redRaw) },
        cellClass: row => garrisonCell(row.value).red ? 'occupation-garrison-alert' : undefined },
      { key: 'law', header: 'Occupation law', label: 'Occupation law', renderMissing: true, width: 'minmax(200px, 1.2fr)', priority: 4,
        text: lawName, value: row => row.value.law === null ? null : lawName(row),
        render: row => { const token = row.value.law; const law = token === null ? undefined : laws.get(token);
          return <span className="occupation-law" data-tooltip={token === null ? row.value.lawMissing ?? undefined : contentTitleHint(token, law?.missing)}>
            {law?.icon ? <img src={law.icon} alt="" /> : null}<span>{lawName(row)}</span></span>; } },
    ];
  }, [names, labelAt, laws, redRaw, namesHour]);
  const tooltip = useCallback((row: OccupiedRow) => {
    const label = labelAt(namesHour);
    const content = occupationTooltip(row, live.read(), tag => label(tag).name, data?.strengthMissing ?? '');
    return fitTooltip([{ ...content, ...countryNameDetails(row.value.occupied === null ? null : label(row.value.occupied)) }], viewportHeight);
  }, [labelAt, namesHour, live, data?.strengthMissing, viewportHeight]);
  const warnings = data?.warnings ?? NO_WARNINGS;
  useWhyCommit('OccupiedStatesWidget', { data, at: head.at, read: head.read, drawn, namesHour, labels: labelAt, names, groups: groups.data });
  useEffect(() => {
    onDiagnostics(warnings);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, onDiagnostics]);

  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign to see occupied states." />;
  else if (head.read === 'off') body = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (head.error && !data) body = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (needsGroups && groups.error && !groups.data) body = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (!data || !match.ready || !view) body = <WidgetState kind="loading" label="Loading occupied states…" />;
  else if (data.missing !== null) body = <WidgetState kind="unavailable" message={`Occupied states weren’t recorded at this time: ${data.missing}`} />;
  else if (!rows.length && view.membership) body = <WidgetState kind="unavailable" message={view.membership} />;
  else if (!rows.length && view.selectionUnknown) body = <WidgetState kind="unavailable" message="Occupied states unavailable for these countries." />;
  else if (!rows.length) body = <WidgetState kind={data.complete ? 'empty' : 'unavailable'} message={data.complete ? 'No occupied states match these countries at this moment.' : 'Some occupied states are missing.'} />;
  else body = <>
    {!data.complete && <p className="occupation-note">Some occupied states are missing.</p>}
    {view.selectionUnknown && <p className="occupation-note">Some states couldn’t be matched to these countries.</p>}
    <Table rowRevision={row => [row.value, columns]} key={`${track.source.id}:${track.branch.id}`} label="Occupied states" rows={rows} rowKey={rowKey} columns={columns} sort={sort}
      setOption={setOption} cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true} density={options.density === 'compact' ? 'compact' : 'comfortable'}
      narrow={narrowTableOf(options)} menu={() => [jumpItem(runtime, track.id, hour)]} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} live={tableLive} />
  </>;
  return <div className="occupied-states" ref={head.root}>
    {needsGroups && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {body}
  </div>;
}
