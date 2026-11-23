import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo,  useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import Legend from '../../charts/Legend';
import WindowedRows from '../../charts/WindowedRows';
import RunLines from '../../charts/RunLines';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { liveKey } from '../../charts/live';
import type { LiveValues } from '../../charts/live';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { nameReadText } from '../../dashboard/nameReads';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { dayLabel } from '../../charts/draw';
import { hatchStyle } from '../../charts/hatch';
import { jumpItem } from '../../charts/jump';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { contentKey, contentSet, resolveOptions } from '../../dashboard/options';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import type { ReadCoverage } from '../../dashboard/widgetTime';

import { useProduct } from '../../dashboard/products';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { historyRuns, minefieldsStructure, minesText, orderRegions, ownerMines, regionRows, regionTotalText, segmentWidths, validateHistory, validateMinefields,
  validateReports } from './minefieldsData';
import type { RegionRow } from './minefieldsData';
import { OPTIONS } from './definition';
import './minefields.css';
import type { MineReport, MinefieldReportsReply } from '../../generated/dashboard';
import {  type RegionAction } from '../../dashboard/regionTime.ts';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const reportKey = (report: MineReport) => `${report.kind}:${report.ordinal}`;

export default function MinefieldsWidget({ widget, runtime, active, onDiagnostics, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tags = selectedCountryKeys(widget.countries, track);
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const anchor = useRef<HTMLDivElement | null>(null);
  const menu = useContextMenu('Minefields');
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'minefields', useMemo(() => tags.length ? { countries: tags.map(countryFromKey) } : {}, [JSON.stringify(tags)]),
    { active, validate: validateMinefields, structure: minefieldsStructure });
  const { data, error, retry } = head;
  const live = head.live;
  const attach = useCallback((element: HTMLDivElement | null) => { anchor.current = element; head.root(element); }, [head.root]);
  const mainAt = useRegionHour(runtime, useRegion(widget), active);
  const shownHour = () => head.at ?? head.hour ?? mainAt;
  useShownEachFrame(runtime.clock, () => data ? head.shownAt() : null, head.live);
  const chosen = contentSet(options.regions).map(contentKey);
  const shown = useMemo(() => data ? regionRows(data, chosen) : null, [data, JSON.stringify(chosen)]);
  const rows = useMemo(() => shown ? orderRegions(shown.rows, options.order === 'name' ? 'name' : 'mines') : [], [shown, options.order]);
  const owners = useMemo(() => [...new Set(rows.flatMap(row => row.owners.map(owner => owner.tag)))], [rows]);
  const lookHour = shownHour() ?? 0;
  const looks = useCountryLooks(runtime.client, track, owners, lookHour, active);
  const colour = useCountryColours(owners, looks.color, tokens, looks);
  useLayoutEffect(() => {
    if (!data) return undefined;
    const paint = () => {
      const values = live.read(), widths = segmentWidths(rows, values);
      for (const node of anchor.current?.querySelectorAll<HTMLElement>('[data-mines]') ?? []) {
        const row = rows.find(item => item.region === node.dataset.mines);
        const text = row ? regionTotalText(row, values, data.scale) : '';
        if (node.textContent !== text) node.textContent = text;
      }
      for (const node of anchor.current?.querySelectorAll<HTMLElement>('[data-segment]') ?? []) {
        const [region, index] = node.dataset.segment!.split('|');
        const width = `${widths.get(region)?.[Number(index)] ?? 0}%`;
        if (node.style.width !== width) node.style.width = width;
      }
    };
    paint();
    return live.subscribe(paint);
  });
  const legend = useMemo(() => owners.map(tag => ({ id: tag, label: looks.name(tag, lookHour), color: colour(tag, lookHour) })), [owners, looks, colour, lookHour]);
  const tooltip = useCallback((key: string): TooltipContent | null => {
    const row = rows.find(item => item.region === key);
    return row && data ? regionTooltip(row, live.read(), tag => looks.name(tag, lookHour), tag => colour(tag, lookHour), data.scale, dayLabel(lookHour)) : null;
  }, [rows, data, looks, colour, lookHour, live]);
  const tips = useRowTooltips<string>({ owner: `minefields:${widget.id}`, active, content: tooltip, revision: `${lookHour}`,
    onMenu: (_, x, y) => { const at = shownHour(); if (track && at !== null) menu.open(x, y, [jumpItem(runtime, track.id, at)]); } });
  const [picked, setPicked] = useState<RegionRow | null>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drillTime = drillControl.region.action;
  const drillRegion = useRegion(widget, drillTime);
  const drillOpen = picked !== null && drill.breakdown !== null;
  const drillAt = useRegionHour(runtime, drillRegion, active && drillOpen);
  useEffect(() => { setPicked(null); dispatch({ type: 'close' }); }, [track?.source.id, track?.branch.id]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  const mined = rows.filter(row => row.owners.length).length;
  const more = !!shown && (shown.unlisted > 0 || rows.some(row => !row.owners.length));
  const pill = headerSlot && data && createPortal(<span className="pill">{mined}{more ? '+' : ''} {mined === 1 && !more ? 'region' : 'regions'} mined</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see minefields." />;
  const parent = head.outside ? <WidgetState kind="outside" read={head} /> : error ? <WidgetState kind="error" failure={error} onRetry={retry} />
    : !data ? <WidgetState kind="loading" label="Loading minefields…" /> : null;
  if (parent && !drillOpen) return error ? parent : <div ref={attach} className="minefields">{parent}</div>;
  const main = parent ?? <div className="minefields-rows">
    <WindowedRows items={rows.map(row => ({ key: row.region, item: row }))} rowHeight={38} fit minRowHeight={16} className="minefields-scroll" label="Minefields by sea region"
      empty={shown?.unlisted ? <WidgetState kind="unavailable" message="Some countries' minefields were not recorded at this date." />
        : <WidgetState kind="empty" message="No minefields at this date." />}>
      {({ item: row }) => <div className="minefields-row" {...tips.rowProps(row.region)} onClick={() => {
        setPicked(row);
        const at = shownHour();
        if (at !== null) dispatch({ type: 'open', breakdown: 'region', hour: at, follow: drillControl.followsAt(at) });
      }}>
        {tips.hovered === row.region && <span id={tips.descriptionId(row.region)} className="sr-only">{tooltipText(tooltip(row.region)!)}</span>}
        <span className="minefields-name">{row.name}</span>
        <div className="minefields-bar">
          {row.owners.map((owner, index) => <span key={owner.tag} className="minefields-segment" data-segment={`${row.region}|${index}`}
            style={{ width: '0%', background: colour(owner.tag, lookHour) }} />)}
          {row.unknownOwners > 0 && <span className="minefields-segment" style={{ ...hatchStyle(tokens), flex: '1 1 0' }} aria-label="Some owners not recorded" />}
        </div>
        <span className="minefields-total" data-mines={row.region}>{data ? regionTotalText(row, live.read(), data.scale) : ''}</span>
      </div>}
    </WindowedRows>
    <Legend label="Mine owners" items={legend} always windowed />
  </div>;
  return <div ref={attach} className="minefields">
    {pill}
    <DrillHost drill={drillControl} runtime={runtime} subject={drillOpen} anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} title={picked?.name ?? 'Region'}
      day={drillAt === null ? '' : dayLabel(drillAt)} canFollow
      onClose={() => { setPicked(null); dispatch({ type: 'close' }); }} main={main}>
      {picked && <RegionDrill widget={widget} runtime={runtime} active={active} region={picked} hoursPerSecond={hoursPerSecond}
        name={tag => looks.name(tag, lookHour)} colour={tag => colour(tag, lookHour)} time={drillTime}
        onPick={hour => dispatch({ type: 'pick', hour })} />}
    </DrillHost>
    {menu.menu}
  </div>;
}

function RegionDrill({ widget, runtime, active, region, hoursPerSecond, name, colour, time, onPick }: Pick<WidgetProps, 'widget' | 'runtime' | 'active'> & {
  region: RegionRow; hoursPerSecond: number | undefined; name: (tag: string) => string; colour: (tag: string) => string;
  time: RegionAction; onPick: (hour: number) => void;
}) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0])!;
  const menu = useContextMenu('Minefields');
  const tags = region.owners.map(owner => owner.tag);
  const gate = useGate(runtime, widget, time);
  const span = gate.span(track.branch.firstHour, track.branch.lastHour);
  const history = useProduct(runtime.client, tags.length && span ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'minefield-history',
    params: { branchId: track.branch.id, region: Number(region.region), countries: tags, ...span } }) : null,
  active, validateHistory, track.source);
  const reports = useHeadProduct(runtime, gate, 'minefield-reports', { region: Number(region.region) }, { active, validate: validateReports });
  const namedAt = reports.at;
  const reportTags = useMemo(() => namedAt === null || !reports.data ? [] : [...new Set(reports.data.reports.flatMap(report => [report.victim, report.mineOwner])
    .flatMap(tag => tag === null ? [] : [tag]))].map(tag => ({ hour: namedAt, tag })), [reports.data, namedAt]);
  const reportNames = useCountryNamesAt(runtime.client, track, reportTags, active);
  const reportName = useCallback((tag: string) => namedAt === null ? name(tag) : nameReadText(reportNames.get(nameKey(namedAt, tag))), [reportNames, namedAt, name]);
  const lines = useMemo(() => (history.data?.lines ?? []).map(line => { const tag = countryKey(line.country); return { id: tag, label: name(tag), color: colour(tag), runs: historyRuns(line, history.data!.scale) }; }),
    [history.data, name, colour]);
  return <div className="minefields-drill" ref={reports.root}>
    <div className="minefields-history">{history.error ? <WidgetState kind="error" what="the region’s history" failure={history.error} onRetry={history.retry} />
      : <RunLines label={`Mines in ${region.name} by owner`} extent={[track.branch.firstHour, track.branch.lastHour]} branchStart={track.branch.firstHour} playhead={null}
        hour={reports.at} lines={lines} active={active} revision={lines} hoursPerSecond={hoursPerSecond}
        formatValue={value => value.toLocaleString('en-US', { maximumFractionDigits: 2 })} empty="Mines not recorded" onPick={hour => onPick(Math.floor(hour))}
        onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, at)])} />}</div>
    <ReportTable reply={reports.data} error={reports.error} read={reports} hoursPerSecond={hoursPerSecond} name={reportName} />
    {menu.menu}
  </div>;
}

function regionTooltip(row: RegionRow, values: LiveValues<number>, name: (tag: string) => string, colour: (tag: string) => string, scale: number, read: string): TooltipContent {
  const amount = (tag: string) => { const mines = ownerMines(values, row.region, tag); return mines === null ? values.missing(liveKey(row.region, tag)) : minesText(mines, scale); };
  return {
    title: row.name, scrollable: true, scrollId: row.region,
    rows: [
      { label: 'Mines', value: regionTotalText(row, values, scale) },
      ...row.owners.map(owner => ({ label: name(owner.tag), swatch: colour(owner.tag), value: amount(owner.tag), indent: true })),
      ...row.unknownOwners ? [{ label: 'Not recorded', value: `${row.unknownOwners} ${row.unknownOwners === 1 ? 'country' : 'countries'}` }] : [],
      { label: 'As of', value: read },
    ],
  };
}

function ReportTable({ reply, error, read, name, hoursPerSecond }: { reply?: MinefieldReportsReply; error?: import('../../dashboard/queryError').Failure; read: ReadCoverage; name: (tag: string) => string; hoursPerSecond: number | undefined }) {
  const country = useCallback((tag: string | null) => tag === null ? 'Country unresolved' : name(tag), [name]);
  const columns = useMemo<readonly TableColumn<MineReport>[]>(() => [
    { key: 'ship', header: 'Ship', label: 'Ship', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.ship, render: row => row.ship ?? 'Ship name unavailable', renderMissing: true },
    { key: 'victim', header: 'Country', label: 'Country', width: 'minmax(80px, 1fr)', priority: 2, value: row => country(row.victim) },
    { key: 'owner', header: 'Mines of', label: 'Mines of', width: 'minmax(80px, 1fr)', priority: 1, value: row => country(row.mineOwner) },
    { key: 'sunk', header: 'Sunk', label: 'Sunk', width: 'max-content', value: row => row.sunk ? 'Sunk' : 'Damaged' },
  ], [country]);
  if (error) return <WidgetState kind="error" what="the region’s reports" failure={error} />;
  if (!reply) return <WidgetState kind="loading" label="Loading reports…" read={read} />;
  if (!reply.reports.length) return reply.readHour === null ? <WidgetState kind="unavailable" message="Standing reports in this region were not recorded at this date." />
    : <p className="drill-empty">No standing reports in this region at this date.</p>;
  return <div className="minefields-reports"><Table rowRevision={row => [row, columns]} label="Standing reports in this region" rows={reply.reports} columns={columns} rowKey={reportKey}
    unsorted sort={{ column: 'ship', direction: 'ascending' }} hoursPerSecond={hoursPerSecond}
    tooltip={row => ({ title: row.ship ?? 'Ship name unavailable', rows: [
      { label: 'Country', value: country(row.victim) }, { label: 'Mines of', value: country(row.mineOwner) },
      { label: 'Equipment', value: row.equipment ?? 'Equipment name unavailable' },
      { label: 'Outcome', value: row.sunk ? 'Sunk' : 'Damaged' },
    ] })} /></div>;
}
