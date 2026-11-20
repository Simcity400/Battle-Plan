import { countryFromKey } from '../../dashboard/countryKey.ts';
import { failureDetails, type Failure } from '../../dashboard/queryError.ts';
import { nameReadText, nameReadFailure } from '../../dashboard/nameReads.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf, tableSortToken } from '../../charts/table/options';
import LiveText, { liveTexts } from '../../charts/LiveText';
import { liveKey, mapLive } from '../../charts/live';
import type { Live, LiveValues } from '../../charts/live';
import type { SortValue } from '../../charts/table/logic';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import { contentKey, contentSet } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { headRead, useHeadProduct } from '../../dashboard/headProduct';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { useShownHour } from '../../dashboard/widgetPerf';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useCountryNamesAt, nameKey } from '../../dashboard/countryNamesAt';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { DEFAULT_SORT } from './definition';
import { efficiencyText, flownBy, missionRows, missionTooltip, missionValues, openWings, regionRows, shownMissions, theatreRegions, validateAirWings } from './airMissionsData';
import type { MissionRow } from './airMissionsData';
import type { AirMissionWing, AirMissionWingsReply } from '../../generated/dashboard';
import { useAirMissions } from './useAirMissions';
import './airMissions.css';

const rowKey = (row: MissionRow) => row.key;
const wingKey = (row: AirMissionWing) => row.id;
const NO_ROWS: readonly MissionRow[] = [];
const count = (value: NonNullable<SortValue>) => typeof value === 'string' ? value : value.toLocaleString('en-US');
const wingsStructure = (reply: AirMissionWingsReply) => JSON.stringify([reply.wings, reply.missing, reply.warnings]);

/** Where a country's air wings are assigned at the hour shown: planes per mission in each strategic region, wings
 * counted once per region. Following playback it is a pushed panel: it renders when the regions, their missions or
 * the theatres change, and every count is painted from its live cell each frame. */
export default function AirMissionsWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const anchor = useRef<HTMLDivElement | null>(null);
  const { track, tag, options, data, error, retry, root, read, live, shownAt, at } = useAirMissions(widget, runtime, active);
  const shown = useShownHour();
  const shownNow = useRef(shownAt);
  shownNow.current = shownAt;
  const chosenRegions = contentSet(options.regions).map(contentKey);
  const chosenTheatres = contentSet(options.theatres).map(contentKey);
  const chosenMissions = contentSet(options.missions).map(contentKey);
  const allRows = useMemo(() => data ? missionRows(data) : NO_ROWS, [data]);
  const theatresKnown = !chosenTheatres.length || !!data?.theatres;
  const inTheatres = theatreRegions(data?.theatres, chosenTheatres);
  const regionKey = JSON.stringify([chosenRegions, inTheatres && [...inTheatres]]);
  const rows = useMemo(() => {
    const regions = chosenRegions.length ? new Set(chosenRegions.filter(region => !inTheatres || inTheatres.has(region))) : inTheatres;
    return regionRows(allRows, regions);
    // The region choice is rebuilt each render from the same options.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [allRows, regionKey]);
  const missions = useMemo(() => data ? shownMissions(data.missions, rows, chosenMissions) : [],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [data, rows, JSON.stringify(chosenMissions)]);
  // Every frame the table paints its counts reports the hour they are of.
  const values = useMemo(() => mapLive(reporting(live, () => shown(shownNow.current())), from => missionValues(from, allRows)), [live, shown, allRows]);
  const notFlown = useMemo(() => flownBy(allRows), [allRows]);
  const readHour = data?.readHour ?? null;
  const readDate = readHour === null ? null : dayLabel(readHour);
  const icon = useSpriteArtwork(runtime, track, useMemo(() => missions.map(mission => mission.icon), [missions]), active);
  const columns = useMemo<readonly TableColumn<MissionRow>[]>(() => [
    { key: 'region', header: 'Region', label: 'Region', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name,
      render: row => <span className="air-missions-region" data-tooltip={row.nameMissing ?? undefined}>{row.name}</span> },
    ...missions.map((mission, index) => ({ key: `m${mission.id}`, headerRevision: icon(mission.icon), header: <MissionHeader name={mission.name} icon={icon(mission.icon)} />,
      label: mission.name, numeric: true, width: 'max-content', priority: index + 1, inTooltip: true, bar: 'visible' as const, value: () => null, paint: count,
      // A mission a region has no registration for is empty, not unknown.
      missingText: (key: string) => notFlown(key) ? '' : '—' })),
    { key: 'wings', header: 'Wings', label: 'Wings', numeric: true, width: 'max-content', inTooltip: true, value: () => null, paint: count },
  ], [missions, icon, notFlown]);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const tooltip = useCallback((row: MissionRow) => missionTooltip(row, missions, values.read(), readDate), [missions, values, readDate]);
  const menu = useCallback(() => track && readHour !== null ? [jumpItem(runtime, track.id, readHour)] : [], [runtime, track, readHour]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [region, setRegion] = useState<MissionRow | null>(null);
  useEffect(() => { dispatch({ type: 'close' }); setRegion(null); }, [track?.source.id, track?.source.revision, track?.branch.id, tag]);
  const following = widget.time.mode === 'follow';
  const onRowClick = useCallback((row: MissionRow) => {
    // The hour is read when the region is opened, never by a render.
    const hour = headRead(widget, track, runtime.tracks, null).hourAt(runtime.clock.getHour());
    if (hour === null) return;
    setRegion(row);
    dispatch(openWings(hour, following));
  }, [widget, track, runtime, following]);
  const open = drill.breakdown !== null && region !== null;
  const wings = useHeadProduct<AirMissionWingsReply>(runtime, widget, 'air-mission-wings', region && drill.breakdown !== null && tag ? { country: countryFromKey(tag), region: Number(region.region) } : null,
    { active: active && open, validate: validateAirWings, structure: wingsStructure, hoverHour: drill.follow ? null : drill.hour });
  const wingsShown = useRef(wings.shownAt);
  wingsShown.current = wings.shownAt;
  const dayLive = useMemo(() => liveTexts(runtime.clock, () => wingsShown.current() ?? Number.NaN,
    hour => new Map([['day', Number.isFinite(hour) ? dayLabel(hour) : '']])), [runtime.clock]);
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...wings.data?.warnings ?? []] : null);
  const nameHour = at ?? readHour;
  const country = useCountryNamesAt(runtime.client, track, useMemo(() => tag && nameHour !== null ? [{ hour: nameHour, tag }] : [], [tag, nameHour]), active);
  const countryName = tag && nameHour !== null ? nameReadText(country.get(nameKey(nameHour, tag))) : undefined;
  const attach = useCallback((element: HTMLDivElement | null) => { anchor.current = element; root(element); }, [root]);
  const pill = headerSlot && data && !data.missing && theatresKnown && createPortal(<span className="pill">{rows.length} {rows.length === 1 ? 'region' : 'regions'}</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see air missions." />;
  if (!tag) return <WidgetState kind="empty" icon="plane" message="Choose a country in the widget's header." />;
  if (read === 'off' && !data) return <div ref={attach}><WidgetState kind="unavailable" message="Choose another time in widget settings." /></div>;
  if (error) return <div ref={attach}><WidgetState kind="error" failure={error} onRetry={retry} /></div>;
  if (!data) return <div ref={attach}><WidgetState kind="loading" label="Loading air missions…" /></div>;
  const main = data.missing ? <WidgetState kind="unavailable" message={data.missing} />
    : !theatresKnown ? <WidgetState kind="unavailable" message={data.theatresMissing ?? 'Air theatre membership was not recorded at this time.'} />
    : !rows.length ? <WidgetState kind="empty" message={`${countryName ?? 'This country'} has no air missions at this date.`} />
    : <Table rowRevision={row => [row, columns]} key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label="Air missions by region" rows={rows} columns={columns}
      rowKey={rowKey} sort={sort} setOption={setOption} sortOptionValue={tableSortToken} cellBars={options.cellBars === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} tooltip={tooltip} menu={menu}
      hoursPerSecond={hoursPerSecond} onRowClick={onRowClick} live={values} />;
  return <div ref={attach} data-detail={failureDetails([...country.values()].map(nameReadFailure))} className="air-missions">
    {pill}
    <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={open}
      title={region?.name ?? 'Wings'} day={drill.follow ? <LiveText live={dayLive} at="day" /> : drill.hour === null ? '' : dayLabel(drill.hour)}
      following={drill.follow} canFollow={following} onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      <div ref={wings.root} className="air-missions">
        <WingTable reply={wings.data} error={wings.error} missions={data.missions} live={wings.live} hoursPerSecond={hoursPerSecond} />
      </div>
    </DrillHost>
  </div>;
}

/** One region's wings at the drill's hour: followed with the widget, or read at the day picked. Planes are painted
 * from their live cells. */
function WingTable({ reply, error, missions, live, hoursPerSecond }: { reply?: AirMissionWingsReply; error?: Failure; missions: readonly { id: string; name: string }[];
  live: Live<LiveValues<number>>; hoursPerSecond: number | undefined }) {
  const names = useMemo(() => new Map(missions.map(mission => [mission.id, mission.name])), [missions]);
  const missionText = useCallback((row: AirMissionWing) => row.missions.map(id => names.get(id) ?? `Mission ${id}`).join(', '), [names]);
  const columns = useMemo<readonly TableColumn<AirMissionWing>[]>(() => [
    { key: 'name', header: 'Wing', label: 'Wing', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? row.id,
      render: row => row.name ?? 'Wing name unavailable' },
    { key: 'type', header: 'Aircraft', label: 'Aircraft', width: 'minmax(80px, 1fr)', priority: 2, value: row => row.type,
      render: row => row.type ?? 'Aircraft name unavailable', renderMissing: true },
    { key: 'planes', header: 'Planes', label: 'Planes', numeric: true, width: 'max-content', value: () => null, paint: count },
    { key: 'missions', header: 'Missions', label: 'Missions', width: 'minmax(80px, 2fr)', priority: 1, sortable: false, value: missionText },
    { key: 'efficiency', header: 'Efficiency', label: 'Efficiency', numeric: true, width: 'max-content', priority: 3,
      value: row => row.efficiency === null ? null : Number(row.efficiency), render: row => efficiencyText(row.efficiency) },
  ], [missionText]);
  const tooltip = useCallback((row: AirMissionWing) => {
    const planes = live.read().get(liveKey(row.id, 'planes'));
    return { title: row.name ?? 'Wing name unavailable', rows: [
      { label: 'Aircraft', value: row.type ?? 'Aircraft name unavailable' },
      { label: 'Planes', value: planes === undefined ? live.read().missing(liveKey(row.id, 'planes')) : planes.toLocaleString('en-US') },
      { label: 'Missions', value: missionText(row) || 'None' },
      { label: 'Efficiency', value: row.efficiency === null ? 'Not recorded' : efficiencyText(row.efficiency) },
      ...row.refusals.map(why => ({ label: 'Not shown', value: why })),
    ] };
  }, [live, missionText]);
  if (error) return <WidgetState kind="error" what="the region’s wings" failure={error} />;
  if (!reply) return <WidgetState kind="loading" label="Loading wings…" />;
  if (reply.missing) return <WidgetState kind="unavailable" message={reply.missing} />;
  if (!reply.wings.length) return <WidgetState kind="empty" message="No wings are assigned here at this date." />;
  return <Table rowRevision={row => [row, columns]} label="Wings in this region" rows={reply.wings} columns={columns} rowKey={wingKey}
    sort={{ column: 'planes', direction: 'descending' }} hoursPerSecond={hoursPerSecond} tooltip={tooltip} live={live} />;
}

/** A mission column's head: the game's mission icon, when drawable, before its name. */
function MissionHeader({ name, icon }: { name: string; icon: string | null }) {
  return <span className="air-missions-head">{icon && <img src={icon} alt="" />}<span>{name}</span></span>;
}
