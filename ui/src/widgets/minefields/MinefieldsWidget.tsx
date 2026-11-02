import { failureDetails, type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import Legend from '../../charts/Legend';
import WindowedRows from '../../charts/WindowedRows';
import RunLines from '../../charts/RunLines';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { PreparedKey } from '../../charts/PreparedKey';
import { dayLabel } from '../../charts/draw';
import { hatchStyle } from '../../charts/hatch';
import { jumpItem } from '../../charts/jump';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { countryNameDetails, tooltipText } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { useHolderLooks } from '../../dashboard/holderLooks.ts';
import { nameReadMetadata, nameReadFailure } from '../../dashboard/nameReads.ts';
import { chartRuns, countryPart } from '../../dashboard/keyedHistory';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { loadTransientProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import * as requests from '../../dashboard/batch5Requests';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { minesAt, minesText, orderRegions, ownerSeries, regionHolders, regionRuns, regionTotalText, validateReports } from './minefieldsData';
import type { MineReport, RegionMines, ReportsReply } from './minefieldsData';
import { useMinefields } from './useMinefields';
import '../air-superiority-region/airSuperiorityRegion.css';
import './minefields.css';

const reportKey = (report: MineReport) => `${report.kind}:${report.ordinal}`;

/** Who has mined which seas at the hour shown: per sea region, each owner's current mines as
 * segments on one shared scale, with the region's standing reports on demand. */
export default function MinefieldsWidget({ widget, runtime, active, onDiagnostics, headerSlot }: WidgetProps) {
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  // A drill that follows playback follows it whatever the widget's own time.
  const playback = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Minefields');
  const { track, options, data, error, retry } = useMinefields(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  // Playback in this campaign's hours, not the comparison clock's, within its branch.
  const clockHour = track ? Math.max(track.branch.firstHour, Math.min(Math.floor(playback) + track.axisOffset, track.branch.lastHour)) : null;
  const owners = useMemo(() => data ? ownerSeries(data.window) : new Map(), [data]);
  const chosen = contentSet(options.regions).map(contentKey);
  const at = useMemo(() => data ? minesAt(data, owners, hour, chosen) : null, [data, owners, hour, JSON.stringify(chosen)]); // eslint-disable-line react-hooks/exhaustive-deps
  const regions = useMemo(() => at ? orderRegions(at.regions, options.order === 'name' ? 'name' : 'mines') : [], [at, options.order]);
  const readHour = at?.readHour ?? hour;
  const tags = useMemo(() => [...new Set([...owners.values()].map(series => series.tag))], [owners]);
  const looks = useCountryLooks(runtime.client, track, tags, readHour, active);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  const highest = useMemo(() => regions.reduce((max, region) => region.total > max ? region.total : max, 0n), [regions]);
  const legend = useMemo(() => {
    const shown = new Set(regions.flatMap(region => region.owners.map(owner => owner.tag)));
    return [...shown].map(tag => ({ id: tag, label: looks.name(tag, readHour), color: colour(tag, readHour) }));
  }, [regions, looks, colour, readHour]);
  const tooltip = useCallback((key: string): TooltipContent | null => {
    const region = regions.find(item => item.region === key);
    return region && data ? { ...regionTooltip(region, name => looks.name(name, readHour), tag => colour(tag, readHour), data.scale, dayLabel(readHour)),
      ...countryNameDetails(...region.owners.map(owner => ({ nameFailure: looks.nameFailure?.(owner.tag, readHour) }))) } : null;
  }, [regions, data, looks, colour, readHour]);
  const tips = useRowTooltips<string>({ owner: `minefields:${widget.id}`, active, content: tooltip, revision: `${readHour}`,
    onMenu: (_, x, y) => { if (track) menu.open(x, y, [jumpItem(runtime, track.id, readHour)]); } });
  const [picked, setPicked] = useState<RegionMines | null>(null);
  useEffect(() => { dispatch({ type: 'close' }); setPicked(null); }, [track?.source.id, track?.branch.id]);
  const drillAt = drillHour(drill, clockHour);
  const reports = useReports(runtime, track, drill.breakdown ? picked?.region ?? null : null, drillAt, active);
  // Report countries need not own mines; the reply resolves their tags at its asked hour, so they are named then.
  const reportNamedAt = reports.data ? reports.data.hour : null;
  const reportNames = useCountryNamesAt(runtime.client, track, useMemo(() => reportNamedAt === null || !reports.data ? [] : [...new Set(reports.data.reports
    .flatMap(report => [report.victim, report.mineOwner]).flatMap(part => part === null ? [] : [countryPart(part)?.tag ?? part]))]
    .map(tag => ({ hour: reportNamedAt, tag })), [reports.data, reportNamedAt]), active);
  const drillHolders = useMemo(() => picked ? regionHolders(owners, picked.region) : [], [picked, owners]);
  const holders = useHolderLooks(runtime.client, track, drillHolders, active && picked !== null, tokens);
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...reports.data?.warnings ?? []] : null);
  // A region listed, or left out, only because its mines are unknown is not counted, but may be mined.
  const mined = regions.filter(region => region.owners.length).length;
  const more = !!at && (at.unlisted > 0 || regions.some(region => !region.owners.length));
  const pill = headerSlot && at && createPortal(<span className="pill">{mined}{more ? '+' : ''} {mined === 1 && !more ? 'region' : 'regions'} mined</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see minefields." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading minefields…" />;
  if (pendingReplyAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!at) return <WidgetState kind="unavailable" message="Minefields were not recorded at this time." />;
  const prefix = preparedPrefix(data);
  const lines = picked ? [...owners.values()].flatMap(series => series.regions.get(picked.region)?.length
    ? [{ id: series.owner, label: holders.name(series.owner), nameFailure: holders.nameFailure?.(series.owner), color: holders.colour(series.owner), runs: chartRuns(regionRuns(series, picked.region), data.scale) }] : []) : [];
  const main = <div className="air-regions minefields-rows">
    <WindowedRows items={regions.map(region => ({ key: region.region, item: region }))} rowHeight={38} fit minRowHeight={16} className="air-region-scroll" label="Minefields by sea region"
      empty={at.refused || at.unlisted ? <WidgetState kind="unavailable" message="Some countries' minefields were not recorded at this date." />
        : <WidgetState kind="empty" message="No minefields at this date." />}>
      {({ item: region }) => <div className="air-region-row" {...tips.rowProps(region.region)}
        onClick={() => { setPicked(region); dispatch({ type: 'open', breakdown: 'region', hour, playing: widget.time.mode === 'follow',
          playhead: widget.time.mode === 'follow' ? clockHour : null }); }}>
        {tips.hovered === region.region && <span id={tips.descriptionId(region.region)} className="sr-only">{tooltipText(tooltip(region.region)!)}</span>}
        <span className="air-region-name">{region.name}</span>
        <div className="air-region-bar minefields-bar">
          {region.owners.filter(owner => owner.amount > 0n).map(owner => <span key={owner.owner} className="air-region-segment"
            style={{ width: `${highest > 0n ? Number(owner.amount * 10000n / highest) / 100 : 0}%`, background: colour(owner.tag, readHour) }} />)}
          {region.unknownOwners > 0 && <span className="air-region-segment minefields-unknown" style={{ ...hatchStyle(tokens), flex: '1 1 0' }} aria-label="Some owners not recorded" />}
        </div>
        <span className="air-region-wings minefields-total">{regionTotalText(region, data.scale)}</span>
      </div>}
    </WindowedRows>
    <Legend label="Mine owners" items={legend} always windowed />
  </div>;
  return <div ref={anchor} className="minefields" data-detail={failureDetails([...reportNames.values()].map(nameReadFailure))}>
    {pill}
    <PreparedKey pending={prefix?.pending} approximate={prefix?.approximate} />
    <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null}
      title={picked?.name ?? 'Region'} day={drillAt === null ? '' : dayLabel(drillAt)} following={drill.follow} canFollow
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      {picked && <div className="minefields-drill">
        <div className="minefields-history"><RunLines label={`Mines in ${picked.name} by owner`} extent={[track.branch.firstHour, track.branch.lastHour]}
          branchStart={track.branch.firstHour} playhead={null} hour={drillAt} lines={lines} known={data.window.known} active={active}
          revision={lines} hoursPerSecond={hoursPerSecond} formatValue={value => value.toLocaleString('en-US', { maximumFractionDigits: 2 })}
          empty="Mines not recorded" onPick={at => dispatch({ type: 'pick', hour: at })}
          onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, at)])} /></div>
        <ReportTable reply={reports.data} error={reports.error} name={tag => nameReadMetadata(reportNamedAt === null ? undefined : reportNames.get(nameKey(reportNamedAt, tag)))}
          hoursPerSecond={hoursPerSecond} />
      </div>}
    </DrillHost>
    {menu.menu}
  </div>;
}

function regionTooltip(region: RegionMines, name: (tag: string) => string, colour: (tag: string) => string, scale: number, read: string): TooltipContent {
  return {
    title: region.name, scrollable: true, scrollId: region.region,
    rows: [
      { label: 'Mines', value: regionTotalText(region, scale) },
      ...region.owners.map(owner => ({ label: name(owner.tag), swatch: colour(owner.tag), value: minesText(owner.amount, scale), indent: true })),
      ...region.unknownOwners ? [{ label: 'Not recorded', value: `${region.unknownOwners} ${region.unknownOwners === 1 ? 'country' : 'countries'}` }] : [],
      { label: 'As of', value: read },
    ],
  };
}

/** One region's standing reports at the drill's hour, read on demand. */
function useReports(runtime: WidgetProps['runtime'], track: ReturnType<typeof useMinefields>['track'], region: string | null, hour: number | null, active: boolean) {
  const content = useContentRevision(runtime.client);
  const [state, setState] = useState<{ client: WidgetProps['runtime']['client']; key: string; data?: ReportsReply; error?: Failure }>();
  const params = track && region !== null && hour !== null ? requests.minefieldReports(track.branch.id, region, hour) : null;
  const request = track && params ? { sourceId: track.source.id, revision: track.source.revision, product: 'minefields', params } : null;
  const key = JSON.stringify([request, content]);
  useEffect(() => {
    if (!active || !request || !track) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal, track.source).then(reply => {
      if (!controller.signal.aborted) setState({ client: runtime.client, key, data: validateReports(reply) });
    }).catch(error => { if (!controller.signal.aborted) setState({ client: runtime.client, key, error: asFailure(error) }); });
    return () => controller.abort();
    // The request's identity is its key, with the content binding it was read under.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key, active]);
  return state?.client === runtime.client && state.key === key ? state : { key, data: undefined, error: undefined };
}

function ReportTable({ reply, error, name, hoursPerSecond }: { reply?: ReportsReply; error?: Failure; name: (tag: string) => ReturnType<typeof nameReadMetadata>; hoursPerSecond: number | undefined }) {
  const country = useCallback((part: string | null) => part === null ? nameReadMetadata(undefined) : name(countryPart(part)?.tag ?? part), [name]);
  const columns = useMemo<readonly TableColumn<MineReport>[]>(() => [
    { key: 'ship', header: 'Ship', label: 'Ship', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.ship, render: row => row.ship ?? 'Ship name unavailable', renderMissing: true },
    { key: 'victim', header: 'Country', label: 'Country', width: 'minmax(80px, 1fr)', priority: 2, value: row => country(row.victim).name },
    { key: 'owner', header: 'Mines of', label: 'Mines of', width: 'minmax(80px, 1fr)', priority: 1, value: row => country(row.mineOwner).name },
    { key: 'sunk', header: 'Sunk', label: 'Sunk', width: 'max-content', value: row => row.sunk ? 'Sunk' : 'Damaged' },
  ], [country]);
  if (error) return <WidgetState kind="error" what="the region’s reports" failure={error} />;
  if (!reply) return <WidgetState kind="loading" label="Loading reports…" />;
  if (!reply.reports.length) return reply.readHour === null ? <WidgetState kind="unavailable" message="Standing reports in this region were not recorded at this date." />
    : <p className="drill-empty">No standing reports in this region at this date.</p>;
  return <div className="minefields-reports"><Table label="Standing reports in this region" rows={reply.reports} columns={columns} rowKey={reportKey}
    unsorted sort={{ column: 'ship', direction: 'ascending' }} hoursPerSecond={hoursPerSecond}
    tooltip={row => ({ ...countryNameDetails(country(row.victim), country(row.mineOwner)), title: row.ship ?? 'Ship name unavailable', rows: [
      { label: 'Country', value: country(row.victim).name }, { label: 'Mines of', value: country(row.mineOwner).name },
      { label: 'Equipment', value: row.equipment ?? 'Equipment name unavailable' },
      { label: 'Outcome', value: row.sunk ? 'Sunk' : 'Damaged' },
      ...row.firstSeen === null ? [] : [{ label: 'Seen from', value: dayLabel(row.firstSeen) }],
    ] })} /></div>;
}
