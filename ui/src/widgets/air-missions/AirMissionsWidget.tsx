import { failureDetails, type Failure, asFailure } from '../../dashboard/queryError.ts';
import { nameReadText, nameReadFailure } from '../../dashboard/nameReads.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf, tableSortToken } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { PreparedKey } from '../../charts/PreparedKey';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { loadTransientProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import * as requests from '../../dashboard/batch5Requests';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useCountryNamesAt, nameKey } from '../../dashboard/countryNamesAt';
import { DEFAULT_SORT } from './definition';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { efficiencyText, missionRowsAt, missionTooltip, openWings, regionRows, shownMissions, theatreRegions, validateAirWings } from './airMissionsData';
import type { AirWing, AirWingsReply, MissionRow } from './airMissionsData';
import { useAirMissions } from './useAirMissions';
import { useAirTheatres } from './useAirTheatres';
import './airMissions.css';

const rowKey = (row: MissionRow) => row.key;
const wingKey = (row: AirWing) => row.id;
const NO_ROWS: readonly MissionRow[] = [];

/** Where a country's air wings are assigned at the hour shown: planes per mission in each
 * strategic region, wings counted once per region. */
export default function AirMissionsWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const anchor = useRef<HTMLDivElement>(null);
  const { track, tag, options, data, error, retry } = useAirMissions(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const chosenRegions = contentSet(options.regions).map(contentKey);
  const chosenTheatres = contentSet(options.theatres).map(contentKey);
  const theatres = useAirTheatres(runtime, track, 'air-missions', track && tag ? requests.airMissionsHistory(track.branch.id, tag) : null, hour, active && chosenTheatres.length > 0);
  // Filtered by theatre, rows are read at the hour their membership answers.
  const rowsHour = chosenTheatres.length && theatres.hour !== undefined ? theatres.hour : hour;
  const at = useMemo(() => data && tag ? missionRowsAt(data, tag, rowsHour) : null, [data, tag, rowsHour]);
  const theatresKnown = !chosenTheatres.length || !!theatres.theatres;
  const inTheatres = theatreRegions(theatres.theatres ?? undefined, chosenTheatres);
  const regionKey = JSON.stringify([chosenRegions, inTheatres && [...inTheatres]]);
  const rows = useMemo(() => {
    if (!at) return NO_ROWS;
    const regions = chosenRegions.length ? new Set(chosenRegions.filter(region => !inTheatres || inTheatres.has(region))) : inTheatres;
    return regionRows(at.rows, regions);
    // The region choice is rebuilt each render from the same options.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [at, regionKey]);
  const chosenMissions = contentSet(options.missions).map(contentKey);
  const missions = useMemo(() => data ? shownMissions(data.missions, rows, chosenMissions) : [],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [data, rows, JSON.stringify(chosenMissions)]);
  const readHour = at?.readHour;
  const readDate = readHour === undefined ? null : dayLabel(readHour);
  const icon = useSpriteArtwork(runtime, track, useMemo(() => missions.map(mission => mission.icon), [missions]), active);
  const maximum = useMemo(() => rows.reduce((max, row) => missions.reduce((most, mission) => {
    const planes = row.planes.get(mission.id);
    return planes !== undefined && planes > most ? planes : most;
  }, max), 0n), [rows, missions]);
  const columns = useMemo<readonly TableColumn<MissionRow>[]>(() => [
    { key: 'region', header: 'Region', label: 'Region', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name,
      render: row => <span className="air-missions-region">{row.name}</span> },
    ...missions.map((mission, index) => ({ key: `m${mission.id}`, headerRevision: icon(mission.icon), header: <MissionHeader name={mission.name} icon={icon(mission.icon)} />, label: mission.name, numeric: true, width: 'max-content',
      priority: index + 1, inTooltip: true, bar: maximum,
      value: (row: MissionRow) => row.planes.get(mission.id) ?? null,
      // A mission the region has no registration for is empty, not unknown.
      renderMissing: true, render: (row: MissionRow) => row.planes.get(mission.id)?.toLocaleString('en-US') ?? '',
      text: (row: MissionRow) => row.planes.get(mission.id)?.toLocaleString('en-US') ?? 'None' })),
    { key: 'wings', header: 'Wings', label: 'Wings', numeric: true, width: 'max-content', inTooltip: true,
      value: row => row.wings, render: row => row.wings === null ? 'Wings unknown' : row.wings.toLocaleString('en-US'), renderMissing: true },
  ], [missions, maximum, icon]);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const tooltip = useCallback((row: MissionRow) => missionTooltip(row, missions, readDate), [missions, readDate]);
  const menu = useCallback(() => track && readHour !== undefined ? [jumpItem(runtime, track.id, readHour)] : [], [runtime, track, readHour]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [region, setRegion] = useState<MissionRow | null>(null);
  useEffect(() => { dispatch({ type: 'close' }); setRegion(null); }, [track?.source.id, track?.source.revision, track?.branch.id, tag]);
  const drillAt = drillHour(drill, hour);
  const wings = useWings(runtime, track, tag, drill.breakdown ? region?.region ?? null : null, drillAt, active);
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...wings.data?.warnings ?? []] : null);
  const country = useCountryNamesAt(runtime.client, track, useMemo(() => tag && readHour !== undefined ? [{ hour: readHour, tag }] : [], [tag, readHour]), active);
  const countryName = tag && readHour !== undefined ? nameReadText(country.get(nameKey(readHour, tag))) : undefined;
  const pill = headerSlot && at && theatresKnown && createPortal(<span className="pill">{rows.length} {rows.length === 1 ? 'region' : 'regions'}</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see air missions." />;
  if (!tag) return <WidgetState kind="empty" icon="plane" message="Choose a country in the widget's header." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading air missions…" />;
  if (pendingReplyAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!at) return <WidgetState kind="unavailable" message="Air missions were not recorded at this time." />;
  const prefix = preparedPrefix(data);
  const main = !theatresKnown ? theatres.error ? <WidgetState kind="error" what="the air theatres" failure={theatres.error} onRetry={theatres.retry} />
    : theatres.theatres === undefined ? <WidgetState kind="loading" label="Loading air theatres…" />
    : <WidgetState kind="unavailable" message="Air theatre membership was not recorded at this time." />
    : !rows.length ? <WidgetState kind="empty" message={`${countryName ?? 'This country'} has no air missions at this date.`} />
    : <Table rowRevision={row => [row, columns]} key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label="Air missions by region" rows={rows} columns={columns}
      rowKey={rowKey} sort={sort} setOption={setOption} sortOptionValue={tableSortToken} cellBars={options.cellBars === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} tooltip={tooltip} menu={menu}
      hoursPerSecond={hoursPerSecond} onRowClick={row => { setRegion(row); dispatch(openWings(rowsHour, hour, widget.time.mode === 'follow')); }}
      parity={{ complete: true, item: row => ({ region: row.region, planes: [...row.planes].map(([id, planes]) => [id, planes.toString()]), wings: row.wings?.toString() ?? null }) }} />;
  return <div ref={anchor} data-detail={failureDetails([...country.values()].map(nameReadFailure))} className="air-missions">
    {pill}
    <PreparedKey pending={prefix?.pending} approximate={prefix?.approximate} />
    <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null}
      title={region?.name ?? 'Wings'} day={drillAt === null ? '' : dayLabel(drillAt)} following={drill.follow} canFollow
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      <WingTable reply={wings.data} error={wings.error} missions={data.missions} hoursPerSecond={hoursPerSecond} />
    </DrillHost>
  </div>;
}

/** One region's wings at the drill's hour, read on demand. */
function useWings(runtime: WidgetProps['runtime'], track: ReturnType<typeof useAirMissions>['track'], tag: string | null, region: string | null, hour: number | null, active: boolean) {
  const content = useContentRevision(runtime.client);
  const [state, setState] = useState<{ client: WidgetProps['runtime']['client']; key: string; data?: AirWingsReply; error?: Failure }>();
  const params = track && tag && region !== null && hour !== null ? requests.airMissionsWings(track.branch.id, tag, region, hour) : null;
  const request = track && params ? { sourceId: track.source.id, revision: track.source.revision, product: 'air-missions', params } : null;
  const key = JSON.stringify([request, content]);
  useEffect(() => {
    if (!active || !request || !track) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal, track.source).then(reply => {
      if (!controller.signal.aborted) setState({ client: runtime.client, key, data: validateAirWings(reply) });
    }).catch(error => { if (!controller.signal.aborted) setState({ client: runtime.client, key, error: asFailure(error) }); });
    return () => controller.abort();
    // The request's identity is its key, with the content binding it was read under.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key, active]);
  return state?.client === runtime.client && state.key === key ? state : { key, data: undefined, error: undefined };
}

function WingTable({ reply, error, missions, hoursPerSecond }: { reply?: AirWingsReply; error?: Failure; missions: readonly { id: string; name: string }[]; hoursPerSecond: number | undefined }) {
  const names = useMemo(() => new Map(missions.map(mission => [mission.id, mission.name])), [missions]);
  const columns = useMemo<readonly TableColumn<AirWing>[]>(() => [
    { key: 'name', header: 'Wing', label: 'Wing', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? row.id,
      render: row => row.name ?? 'Wing name unavailable' },
    { key: 'type', header: 'Aircraft', label: 'Aircraft', width: 'minmax(80px, 1fr)', priority: 2, value: row => row.type,
      render: row => row.type ?? 'Aircraft name unavailable', renderMissing: true },
    { key: 'planes', header: 'Planes', label: 'Planes', numeric: true, width: 'max-content', value: row => row.planes },
    { key: 'missions', header: 'Missions', label: 'Missions', width: 'minmax(80px, 2fr)', priority: 1, sortable: false,
      value: row => row.missions.map(id => names.get(id) ?? id).join(', ') },
    { key: 'efficiency', header: 'Efficiency', label: 'Efficiency', numeric: true, width: 'max-content', priority: 3,
      value: row => row.efficiency === null ? null : Number(row.efficiency), render: row => efficiencyText(row.efficiency) },
  ], [names]);
  if (error) return <WidgetState kind="error" what="the region’s wings" failure={error} />;
  if (!reply) return <WidgetState kind="loading" label="Loading wings…" />;
  if (!reply.wings.length) return reply.readHour === null ? <WidgetState kind="unavailable" message="This region's wings were not recorded at this date." />
    : <WidgetState kind="empty" message="No wings are assigned here at this date." />;
  return <Table rowRevision={row => [row, columns]} label="Wings in this region" rows={reply.wings} columns={columns} rowKey={wingKey}
    sort={{ column: 'planes', direction: 'descending' }} hoursPerSecond={hoursPerSecond}
    tooltip={row => ({ title: row.name ?? 'Wing name unavailable', rows: [
      { label: 'Aircraft', value: row.type ?? 'Aircraft name unavailable' },
      { label: 'Planes', value: row.planes.toLocaleString('en-US') },
      { label: 'Missions', value: row.missions.map(id => names.get(id) ?? id).join(', ') || 'None' },
      { label: 'Efficiency', value: row.efficiency === null ? 'Not recorded' : efficiencyText(row.efficiency) },
    ] })} />;
}

/** A mission column's head: the game's mission icon, when drawable, before its name. */
function MissionHeader({ name, icon }: { name: string; icon: string | null }) {
  return <span className="air-missions-head">{icon && <img src={icon} alt="" />}<span>{name}</span></span>;
}
