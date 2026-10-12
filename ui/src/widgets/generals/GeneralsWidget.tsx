import { contentTitle } from '../../dashboard/contentTitles.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { jumpItem } from '../../charts/jump';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useLeaderPortraits } from './portraits';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import CareerView from '../general-career/CareerView';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { Attributes, AttributesHeader } from './Attributes';
import { LeaderIdentity, SkillStars } from './LeaderIdentity';
import { leadersAt, leaderName, leaderTooltip, pinnedCareer, attributeNames, recordedSince } from './generalsData';
import type { LeaderRow } from './generalsData';
import { useLeaderCountries, useLeaders } from './useLeaders';
import './generals.css';

const EMPTY: readonly LeaderRow[] = [];
const rowKey = (row: LeaderRow) => row.key;
// Artwork and labels rebuild cells without changing the table's sort identity.
const skillValue = (row: LeaderRow) => row.value.skill;
const wonValue = (row: LeaderRow) => row.won;
const lostValue = (row: LeaderRow) => row.lost;
const unresolvedValue = (row: LeaderRow) => row.unresolved;
const winRateValue = (row: LeaderRow) => row.winRate;

export default function GeneralsWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics, setOption } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const owner = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id]);
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const [drillOwner, setDrillOwner] = useState<object | null>(null);
  const [savedDrill, dispatch] = useReducer(reduceDrill, CLOSED);
  const drill = drillOwner === owner ? savedDrill : CLOSED;
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const playback = useTableHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const selection = useLeaderCountries(widget, runtime, hour, active);
  const options = resolveOptions(OPTIONS, widget.options);
  const { data, error, retry } = useLeaders(runtime, widget.trackIds[0], selection.requestSelection, 'leaderboard', active);
  const rows = useMemo(() => data && selection.countries !== undefined ? leadersAt(data, hour, selection.countries, String(options.leaders)) : EMPTY, [data, hour, selection.countries, options.leaders]);
  const [visible, setVisible] = useState<readonly LeaderRow[]>(EMPTY);
  const portraits = useLeaderPortraits(runtime, track, visible, 86, active && drill.breakdown == null);
  const attributeMaxima = useMemo(() => [0, 1, 2, 3].map(i => rows.reduce((max, row) => Math.max(max, row.value.components[i] ?? 0), 0)), [rows]);
  const wanted = useMemo(() => visible.flatMap(row => row.value.owner ? [{ hour: row.hour, tag: row.value.owner }] : []), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const viewportHeight = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  const open = useCallback((row: LeaderRow) => {
    if (currentOwner.current !== owner) return;
    setDrillOwner(owner);
    dispatch({ type: 'open', breakdown: row.key, hour: row.hour,
      playing: widget.time.mode === 'follow' && runtime.clock.getSnapshot().playing,
      playhead: widget.time.mode === 'follow' ? hour : null });
  }, [owner, runtime.clock, hour, widget.time.mode]);
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(null); }, [owner]);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? []), ...portraits.warnings])], [data, portraits.warnings]);
  // An open career reports its own warnings; closing it sends the leaderboard's again.
  useDiagnostics(onDiagnostics, drill.breakdown == null ? diagnostics : null);
  const columns = useMemo<readonly TableColumn<LeaderRow>[]>(() => {
    return [
      { key: 'leader', header: 'Leader', label: 'Leader', sortable: false, width: 'minmax(190px, 2fr)', value: leaderName,
        render: row => data && <LeaderIdentity art={portraits.at(row)} row={row} country={row.value.owner ? labelAt(row.hour)(row.value.owner) : { name: 'Owner not recorded' }} /> },
      { key: 'skill', header: 'Skill', label: 'Skill', width: '92px', value: skillValue,
        renderMissing: true, render: r => <SkillStars level={r.value.skill} maximum={r.value.maximumSkill} /> },
      { key: 'attributes', header: <AttributesHeader names={attributeMaxima.map((_, i) =>
          [...new Set(rows.map(row => attributeNames(row.value)[i]))].join(' / ') || 'Name not recorded')} />,
        label: 'Attributes', sortable: false, width: '150px', value: () => null, renderMissing: true,
        render: row => <Attributes values={row.value.components} maxima={attributeMaxima} names={attributeNames(row.value)} bars={options.cellBars === true} /> },
      { key: 'traits', header: 'Traits', label: 'Traits', sortable: false, width: '1.1fr', value: r => r.value.traits?.length,
        renderMissing: true, render: r => <span className="leader-traits">{r.value.traits == null ? <span className="leader-missing">Not recorded</span> : r.value.traits.map(token => {
          const trait = r.traitCatalogue[token]; const name = contentTitle(trait?.name, trait?.token ?? token, 'Trait name unavailable');
          return trait?.icon ? <img key={token} src={trait.icon} alt={name} /> : <span className="leader-trait-chip" key={token}>{name}</span>;
        })}</span> },
      { key: 'won', header: 'Won', label: 'Battles won', numeric: true, width: '64px', value: wonValue, renderMissing: true,
        render: r => r.won ?? <span className="leader-missing" data-tooltip="Result not recorded">—</span> },
      { key: 'lost', header: 'Lost', label: 'Battles lost', numeric: true, width: '64px', value: lostValue, renderMissing: true,
        render: r => r.lost ?? <span className="leader-missing" data-tooltip="Result not recorded">—</span> },
      { key: 'unresolved', header: 'Unresolved', label: 'Unresolved battles', numeric: true, width: '88px', value: unresolvedValue,
        render: r => <span data-tooltip="Battles with no known result.">{r.unresolved}</span> },
      { key: 'fought', header: 'Fought', label: 'Fought', sortable: false, numeric: true, width: '64px', value: r => r.fought, renderMissing: true,
        render: r => <span data-tooltip={recordedSince(r.career, data!.firstHour)}>{r.fought}</span> },
      { key: 'winRate', header: 'Win %', label: 'Win rate among decided battles', numeric: true, width: '56px', value: winRateValue, renderMissing: true,
        render: r => r.winRate == null ? <span className="leader-missing" data-tooltip={r.won == null ? "Result not recorded" : "No battles with known results."}>—</span> : <span data-tooltip="Win rate among decided battles">{Math.round(r.winRate)}%</span> },
    ];
  }, [data, labelAt, portraits, attributeMaxima, options.cellBars, rows]);
  const tooltip = useCallback((row: LeaderRow) => fitTooltip([leaderTooltip(row, row.value.owner ? labelAt(row.hour)(row.value.owner) : { name: 'Owner not recorded' })], viewportHeight), [labelAt, viewportHeight]);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see its leaders." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (selection.group.error) return <WidgetState kind="error" detail={selection.group.error} onRetry={selection.group.retry} />;
  if (selection.selectionReason) return <WidgetState kind="unavailable" message={selection.selectionReason} />;
  if (!data) return <WidgetState kind="loading" label="Loading leaders…" />;
  const observed = data.observed.find(r => r.firstHour <= hour && hour <= r.lastHour);
  const main = selection.countries === undefined ? <WidgetState kind="unavailable" message="Country group membership was not recorded at this hour." />
    : !rows.length && !observed?.leaders ? <WidgetState kind="unavailable" message="Leaders weren’t recorded at this time." />
    : !rows.length ? <WidgetState kind="empty" message="No leaders for these countries at this time." />
    : <div className="leader-table"><Table key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label="Generals leaderboard" rows={rows} columns={columns} rowKey={rowKey} rowHeight={48} rank
      sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} onRowClick={open} tooltip={tooltip} onVisibleRows={setVisible}
      menu={r => [{ ...jumpItem(runtime, track.id, r.career.runs[0].firstHour), label: `Jump to first seen ${formatDateTime(r.career.runs[0].firstHour)}` }, { label: 'Open career', run: () => open(r) }]}
      hoursPerSecond={hoursPerSecond} />
      {!observed?.leaders && <p className="career-note">Some leaders are missing.</p>}
    </div>;
  const clockHour = Math.floor(runtime.clock.getHour()) + track.axisOffset;
  const pickedHour = drillHour(drill, clockHour) ?? hour;
  return <div ref={anchor} className="generals-widget"><DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown != null}
    title="General career" day={formatDateTime(pickedHour)} following={drill.follow} canFollow anchor={anchor}
    onClose={() => { if (currentOwner.current === owner) dispatch({ type: 'close' }); }}
    onFollow={() => { if (currentOwner.current === owner && drill.breakdown) dispatch({ type: 'follow' }); }}
    onPin={drill.breakdown ? () => {
      if (currentOwner.current === owner) runtime.addWidgetBelow(widget.id, pinnedCareer(widget, drill.breakdown!, track, pickedHour, drill.follow));
    } : undefined} main={main}>
    {drill.breakdown && <CareerView {...props} widget={{ ...widget, time: drill.follow ? { mode: 'follow' } : { mode: 'campaign' } }}
      reference={drill.breakdown} hour={pickedHour} onPick={h => {
        if (currentOwner.current === owner) dispatch({ type: 'pick', hour: h });
      }} />}
  </DrillHost></div>;
}
