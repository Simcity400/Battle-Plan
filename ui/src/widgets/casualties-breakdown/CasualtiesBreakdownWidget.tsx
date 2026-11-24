import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { countryTagAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { failureDetails } from '../../dashboard/queryError.ts';
import { nameReadFailure } from '../../dashboard/nameReads.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel, drawBands } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { pinnedBreakdown } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { jumpItem } from '../../charts/jump';
import { stackedSegments, stackExtent } from '../../charts/stack';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryFlagsRead } from '../../dashboard/countryFlags';
import { useCountryNamesAt } from '../../dashboard/countryNamesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { liveAt, mapLive, valuesOf } from '../../charts/live';
import LiveText, { type LiveTexts } from '../../charts/LiveText';
import type { SortValue } from '../../charts/table/logic';
import { useHeadHour } from '../army-size/headHour.ts';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { breakdownWindows, exactLosses, lossDrillAt, lossesAt, lossStack, lossText, validateBreakdown, viewChange, viewWindows, type LossRow } from './casualtiesBreakdownData';
import { casualtyCountryCell, casualtyEnemyName } from './casualtyFlags';
import { definition, DETAIL } from './definition';
import type { CasualtyBreakdownReply } from '../../generated/dashboard';
import { regionHour, axisAt, recordedHour } from '../../dashboard/regionTime.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { breakdownViewOf, levelsShown } from '../../families/breakdown/options.ts';
import { useLaneIdentity } from '../../families/breakdown/useLaneIdentity.ts';
import { casualtiesBreakdownProjection, type CasualtiesBreakdownProjectionOptions } from './casualtiesBreakdownProjection.ts';
import './casualtiesBreakdown.css';

const EMPTY_SUBS = [] as const;
const LEVELS = ['warLevel', 'enemyLevel'];
const SORT = { column: 'losses', direction: 'descending' } as const;
const rowKey = (row: LossRow) => row.id;
const WAR: TableColumn<LossRow> = { key: 'war', header: 'War', label: 'War', priority: 1, inTooltip: true, value: row => row.war };
const LOSSES: TableColumn<LossRow> = { key: 'losses', header: 'Casualties', label: 'Casualties', numeric: true, width: 'max-content', value: row => row.value, render: lossText };

export default function CasualtiesBreakdownWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const content = useContentRevision(runtime.client);
  const speed = usePlaybackMotion(runtime.clock), tokens = useChartTokens(), height = useViewportHeight();
  const options = resolveOptions(definition.options, widget.options);
  const view = breakdownViewOf(options);
  const namesInRing = options.namesInRing !== false;
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Casualty breakdown actions');
  const [visible, setVisible] = useState<readonly LossRow[]>([]);
  const [chartRange, setChartRange] = useState<readonly [number, number] | null>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const drillControl = useDrill(runtime, useRegion(widget), options.show === DETAIL.id ? DETAIL.id : null);
  const drill = drillControl.state;
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const request = useMemo(
    () => track && tag
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'casualty-breakdown',
          params: { branchId: track.branch.id, country: countryFromKey(tag), window: null },
        })
      : null,
    [track?.source.id, track?.source.revision, track?.branch.id, tag],
  );
  const manifest = useProduct<CasualtyBreakdownReply>(runtime.client, request, active, validateBreakdown, track?.source);
  const drillFollows = drill.breakdown !== null && !drill.spec && drillControl.following;
  const drillRows = useRef<() => string>(() => '');
  const headWindow = (at: number) => String(breakdownWindows(manifest.data, at, false)[0] ?? -1);
  const playback = useHeadHour(runtime.clock, active && (widget.time.mode === 'follow' || drillFollows),
    at => drillFollows ? `${headWindow(at)}:${drillRows.current()}` : headWindow(at));
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = window?.lastHour ?? track?.branch.lastHour ?? 0;
  const pinned = options.show === DETAIL.id;
  const open = drillControl.open;
  const clockHour = track ? Math.floor(playback) + track.axisOffset : null;
  const drillRegion = drillControl.region;
  const detailAt = open ? regionHour(drillRegion, track, runtime.tracks, axisAt(playback)) : recordedHour(track, hour);
  const detailHour = detailAt ?? hour;
  const pinSlots = useSlotTags(runtime.client, track, active && !!tag && isSlotKey(tag));
  const pinTag = tag && isSlotKey(tag) ? countryTagAt(pinSlots, countryFromKey(tag), Math.floor(detailHour)) : tag;
  const outside = !!track && (hour < track.branch.firstHour || hour > track.branch.lastHour);
  const detailOutside = !!track && detailAt === null;
  const pickDay = drillControl.pick;
  const currentWindows = breakdownWindows(manifest.data, hour, widget.time.mode === 'follow');
  const detailWindows = breakdownWindows(manifest.data, detailHour, drillControl.following);
  const chartWindows = open && manifest.data ? viewWindows(manifest.data.windows, chartRange) : [];
  const onChartView = useCallback((range: readonly [number, number]) => setChartRange(viewChange(manifest.data?.windows ?? [], range)), [manifest.data]);
  const wanted = [...new Set([...currentWindows, ...detailWindows, ...chartWindows])].sort((a, b) => a - b);
  const identity = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, content]) : null;
  const histories = useProductWindows<CasualtyBreakdownReply>(runtime.client, identity, index => ({ ...request!,
    params: { branchId: track!.branch.id, country: countryFromKey(tag!), window: index } }), wanted, active && manifest.data?.available === true, validateBreakdown, track?.source, true);
  const current = currentWindows[0] === undefined ? undefined : histories.data.get(currentWindows[0]);
  const detail = detailWindows[0] === undefined ? undefined : histories.data.get(detailWindows[0]);
  const detailThrough = detailHour;
  const details = useMemo(() => detail ? lossesAt(detail, detailThrough) : null, [detail, detailThrough]);
  const nameFacts = useMemo(() => (details?.rows ?? []).map(row => ({ tag: row.enemyTag, hour: row.enemyHour })), [details]);
  const names = useCountryNamesAt(runtime.client, track, nameFacts, active && open);
  const enemyName = (row: LossRow) => casualtyEnemyName(row, names);
  const flagFacts = useMemo(() => visible.map(row => ({ tag: row.enemyTag, hour: row.enemyHour })), [visible]);
  const flags = useCountryFlagsRead(runtime.client, track, flagFacts, active && open);
  const { identitySnapshot, onIdentityDemands } = useLaneIdentity(runtime.client, track, active && (!open || options.drill === 'popout'));

  const axisOffset = track?.axisOffset ?? 0;
  const drillLive = useMemo(() => open && drillControl.following ? liveAt(runtime.clock, position => Math.floor(position) + axisOffset, at => {
    const index = breakdownWindows(manifest.data, at, false)[0];
    return lossDrillAt(index === undefined ? undefined : histories.data.get(index), at);
  }) : undefined, [open, drillControl.following, runtime.clock, axisOffset, manifest.data, histories.data]);
  drillRows.current = () => drillLive?.read().rows ?? '';
  const drillValues = useMemo(() => drillLive && mapLive(drillLive, made => valuesOf<SortValue>(made.values, 'Not recorded')), [drillLive]);
  const drillTexts = useMemo<LiveTexts | undefined>(() => drillLive && {
    text: key => key === 'day' ? dayLabel(drillLive.hour()) : drillLive.read().total, subscribe: drillLive.subscribe }, [drillLive]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);
  const chartReplies = useMemo(() => chartWindows.flatMap(index => histories.data.has(index) ? [histories.data.get(index)!] : []),
    [chartWindows.join(','), histories.data]);
  const stack = useMemo(() => lossStack(chartReplies), [chartReplies]);
  const colors = stack.keys.map((_key, index) => tokens.categories[index % tokens.categories.length]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, theme, state) => {
      const until = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      for (const segment of stackedSegments(stack.model, frame.x[0], until, (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth) / 2)) {
        drawBands(ctx, frame, theme, segment.hours, segment.bands, colors);
      }
    },
    extent: range => stackExtent(stack.model, range[0], range[1]),
  }], [stack, colors.join(',')]);
  const columns = useMemo<TableColumn<LossRow>[]>(() => [WAR,
    { key: 'enemy', header: 'Enemy', label: 'Enemy', value: row => casualtyEnemyName(row, names),
      render: row => <CountryCell {...casualtyCountryCell(row, names, flags)} /> },
    { ...LOSSES, paint: (value, key) => lossText({ value: BigInt(value), incomplete: drillLive?.read().incomplete.has(key.split('\u0000')[0]) ?? false }) },
  ], [names, flags, drillLive]);
  useEffect(() => { drillControl.close(); setVisible([]); setChartRange(null); }, [runtime.client, identity, widget.time, options.show]);
  const diagnostics = useMemo(() => [...new Set([...(manifest.data?.warnings ?? []), ...[...histories.data.values()].flatMap(reply => reply.warnings)])],
    [manifest.data, histories.data]);
  useDiagnostics(onDiagnostics, diagnostics);

  const projectionOptions = useMemo<CasualtiesBreakdownProjectionOptions>(() => ({ levels }), [levels]);
  const staticSources = useMemo(() => {
    if (!current || !track) return null;
    const winSpan = current.window !== null ? current.windows[current.window] : undefined;
    return {
      panels: { casualties: current },
      hour: winSpan?.firstHour ?? track.branch.firstHour,
      endHour: winSpan?.lastHour ?? track.branch.lastHour,
    };
  }, [current, track]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : null,
    [track],
  );

  const contextMenuItems = useCallback((_key: string | null, _name: string | null, atHour: number) => {
    if (!track) return [];
    return [
      { label: 'All casualties', run: () => drillControl.show(DETAIL.id, atHour) },
      jumpItem(runtime, track.id, atHour),
    ];
  }, [track, runtime, drillControl]);

  if (!track || !tag) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its casualties." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (manifest.error) return <WidgetState kind="error" failure={manifest.error} onRetry={manifest.retry} />;
  if (!manifest.data) return <WidgetState kind="loading" label="Loading casualty breakdown…" />;
  if (!manifest.data.available) return <WidgetState kind="unavailable" message="Casualties by war were not recorded." />;

  const currentError = currentWindows[0] === undefined ? undefined : histories.errors.get(currentWindows[0]);
  const error = detailWindows[0] === undefined ? undefined : histories.errors.get(detailWindows[0]);
  const summary = (value: typeof details) => value
    ? <p className="casualty-breakdown-total">Casualties: <b>{exactLosses(value.total)}</b></p> : null;
  const drillSummary = (value: typeof details) => drillTexts ? <p className="casualty-breakdown-total"><b><LiveText live={drillTexts} at="total" /></b></p> : summary(value);
  const dayShown = drillTexts ? <LiveText live={drillTexts} at="day" /> : dayLabel(detailHour);
  const detailBody = <div className="casualty-breakdown-details">
    {pinned && <p className="casualty-breakdown-total">{dayShown} {!drillControl.following
      && <button type="button" className="button small" onClick={drillControl.follow}>Follow playback</button>}</p>}
    {detailOutside ? <WidgetState kind="outside" />
      : error ? <WidgetState kind="error" failure={error} onRetry={histories.retry} />
      : !detail ? <WidgetState kind="loading" label="Loading this moment…" />
        : !details ? <p className="drill-empty">{detail.observed.some(span => span.firstHour <= detailThrough && detailThrough <= span.lastHour)
          ? 'No casualty relations were recorded for this country' : 'Casualty counters were not recorded'} at {dayLabel(detailHour)}.</p>
          : <>{drillSummary(details)}<div className="casualty-breakdown-table"><Table rowRevision={row => [row, columns]} label="Recorded casualty lifetimes" rows={details.rows}
            columns={columns} rowKey={rowKey} sort={SORT} live={drillValues}
            cellBars={false} rankMarkers={false} hoursPerSecond={speed} onVisibleRows={setVisible} narrow={narrowTableOf(options)}
            menu={row => [jumpItem(runtime, track.id, row.readHour)]}
            tooltip={row => fitTooltip([{ title: `${row.war} › ${enemyName(row)}`, rows: [
              { label: 'Casualties', value: lossText(row), strong: true },
              { label: 'Enemy key', value: row.enemyTag },
            ] }], height)} /></div></>}
    <div className="casualty-breakdown-history"><TimeChart label="Recorded casualties by war over time"
      extent={[track.branch.firstHour, track.branch.lastHour]} branchStart={track.branch.firstHour}
      playhead={drillControl.following ? clockHour : null} playback="cursor" hoursPerSecond={speed}
      axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers}
      active={active && open} revision={stack} onViewChange={onChartView}
      onPick={pickDay}
      onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, Math.floor(at))])}
      tooltip={at => {
        const index = breakdownWindows(manifest.data, at, false)[0];
        const reply = index === undefined ? undefined : histories.data.get(index);
        const value = reply ? lossesAt(reply, at) : null;
        return { title: dayLabel(Math.floor(at)), rows: [{ label: 'Casualties',
          value: value ? exactLosses(value.total) : reply ? 'Not recorded' : 'Loading…', strong: true }] };
      }} /></div>
    {chartWindows.some(index => !histories.data.has(index) && !histories.errors.has(index)) && <p className="drill-empty">Loading history in the visible time range…</p>}
    {chartWindows.some(index => histories.errors.has(index)) && <button type="button" className="button small" onClick={histories.retry}>Retry history</button>}
  </div>;

  const mainBlocked = outside ? <WidgetState kind="outside" />
    : currentError ? <WidgetState kind="error" failure={currentError} onRetry={histories.retry} />
      : !current ? <WidgetState kind="loading" label="Loading casualty history…" />
        : null;

  const main = (
    <div className="casualty-breakdown-main">
      <FamilyBreakdownHost
        declaration={casualtiesBreakdownProjection}
        options={projectionOptions}
        view={view}
        namesInRing={namesInRing}
        label="Casualties by war and enemy"
        subject="casualties"
        widget={widget}
        runtime={runtime}
        active={active && (!open || options.drill === 'popout')}
        subscriptions={EMPTY_SUBS}
        staticSources={staticSources}
        offset={track.axisOffset}
        pin={widget.time.mode === 'follow' ? null : Math.floor(hour)}
        bounds={bounds}
        identitySnapshot={identitySnapshot}
        onIdentityDemands={onIdentityDemands}
        contextMenuItems={contextMenuItems}
        onSetOption={setOption}
        onDiagnostics={onDiagnostics}
        blockedState={mainBlocked}
        loadingLabel="Loading casualty history…"
      />
    </div>
  );

  return (
    <div ref={anchor} className="casualty-breakdown" data-detail={failureDetails([...names.values()].map(nameReadFailure))}>
      {pinned ? detailBody : (
        <DrillHost
          drill={drillControl}
          runtime={runtime}
          mode={options.drill === 'popout' ? 'popout' : 'inline'}
          title="Recorded casualties"
          day={dayShown}
          canFollow
          pinRefused={drillControl.pinRefused}
          onPin={pinTag && drillControl.time() ? () => {
            const time = drillControl.time();
            if (!time) return;
            const pin = pinnedBreakdown(widget, DETAIL, track.id, time);
            runtime.addWidgetBelow(widget.id, { ...pin, countries: { kind: 'tags', tags: [pinTag] } });
          } : undefined}
          anchor={anchor}
          main={main}
        >
          {detailBody}
        </DrillHost>
      )}
      {menu.menu}
    </div>
  );
}
