import { SPEC } from '../../dashboard/regionTime';
import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import HeatChart from '../../charts/heat/HeatChart';
import type { HeatPeriod, HeatRow, HeatTail } from '../../charts/heat/layout';
import { jumpItem } from '../../charts/jump';
import { hatchStyle } from '../../charts/hatch';
import type { Live } from '../../charts/live';
import type { LiveHead } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { mix } from '../../shell/appearance/color';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useAtHead } from '../../dashboard/atHead';
import { defaultCountryKey, selectedCountryKeys } from '../../dashboard/countrySelection';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { airSummaryTooltip, airTooltip, momentRows, regionLabel, sideOrder, validateAirSuperiority } from '../../dashboard/airSuperiority';
import { airMode, airParams, useAirLooks } from '../../dashboard/useAirSuperiority';
import type { AirRankingReply, AirSide, AirTimelineReply } from '../../generated/dashboard';
import { STEP, cellAt, dayOf, dayView, followedEnd, headTail, joinTiles, validateAirRanking, validateAirTimeline, viewTiles } from './airTimelineData';
import { OPTIONS } from './definition';
import { stickyLimit, useStickyMembers } from '../../dashboard/follow/stickyMembers.ts';
import './airSuperiorityTimeline.css';
import { productRequest } from '../../dashboard/request.ts';

const NO_SIDES: AirSide[] = [];
const RANKING_MOVES = ['lastHour'] as const;

export default function AirSuperiorityTimelineWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const campaign = track?.source.metadata.campaignKey ?? '';
  const currentCountry = selectedCountryKeys(widget.countries, track)[0] ?? defaultCountryKey(track);
  const mode = useMemo(() => airMode(options, campaign, currentCountry),
    [options.mode, options.viewpoint, options.sides, options.regions, options.theatres, campaign, currentCountry]);
  const needsViewpoint = mode.mode === 'viewpoint' && !mode.viewpoint;
  const following = widget.time.mode === 'follow';
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const position = useAtHead(runtime.clock, active && following, hour => followedEnd(hour, runtime.clock.getSnapshot().playing));
  const window = track && resolveWindow(widget, track, runtime.tracks, following ? position : runtime.clock.getSnapshot().position);
  const firstHour = track ? widget.time.mode === 'range' && window ? window.firstHour : track.branch.firstHour : 0;
  const lastHour = !track || !window ? 0 : widget.time.mode === 'campaign' ? track.branch.lastHour : Math.min(window.lastHour, track.branch.lastHour);
  const beforeBranch = !!track && lastHour < firstHour;
  const gate = useGate(runtime, widget, SPEC, active);
  const span = !beforeBranch ? gate.span(firstHour, lastHour) : null;
  const params = useMemo(() => track && window && span && !needsViewpoint ? { branchId: track.branch.id, ...span, ...airParams(mode),
    ...(mode.regions.length ? {} : { top: Number(options.regionCount) }) } : null, [track, window, span?.firstHour, span?.lastHour, needsViewpoint, mode, options.regionCount]);
  const ranking = useFollowedProduct<AirRankingReply>(runtime.client, track && params ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'air-superiority-ranking', params }) : null, RANKING_MOVES, active, validateAirRanking, track?.source);
  const regionsKey = ranking.data ? JSON.stringify(ranking.data.rows.map(row => [row.region, regionLabel({ id: row.region, name: row.name })])) : null;
  const regions = useMemo(() => regionsKey === null ? [] : (JSON.parse(regionsKey) as [number, string][]).map(([id, name]) => ({ id, name })), [regionsKey]);
  const regionNames = useMemo(() => regions.map(region => String(region.id)), [regions]);
  const tileScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, mode, firstHour]);
  const tileRegions = useStickyMembers(tileScope, regionNames, stickyLimit(Math.max(1, Number(options.regionCount) || regionNames.length)));
  const regionIds = useMemo(() => tileRegions.map(Number).sort((a, b) => a - b), [tileRegions]);
  const tiling = useMemo(() => viewTiles(firstHour, lastHour), [firstHour, lastHour]);
  const tileParams = useMemo(() => track && params && span && regionIds.length ? { branchId: track.branch.id, firstHour: span.firstHour, ...airParams(mode), regions: regionIds } : null,
    [track, params, regionIds, span?.firstHour, mode]);
  const identity = track && tileParams ? JSON.stringify([track.source.id, track.source.revision, tileParams]) : null;
  const tileRequest = useCallback((tile: number) => (productRequest({ sourceId: track!.source.id, revision: track!.source.revision, product: 'air-superiority-timeline',
    params: { ...tileParams!, lastHour: gate.within(tile * STEP + STEP - 1)!, tile } })), [track, tileParams, gate.within]);
  const tiles = useProductWindows<AirTimelineReply>(runtime.client, identity, tileRequest, tiling.whole, active, validateAirTimeline, track?.source);
  const cut = useProduct<AirTimelineReply>(runtime.client, track && tileParams && tiling.cut !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'air-superiority-timeline', params: { ...tileParams, lastHour: span!.lastHour, tile: tiling.cut } }) : null, active, validateAirTimeline, track?.source);
  const order = useMemo(() => [...tiling.whole, ...(tiling.cut === null ? [] : [tiling.cut])], [tiling]);
  const replies = useMemo(() => order.map(tile => tile === tiling.cut ? cut.data : tiles.data.get(tile)), [order, tiling.cut, cut.data, tiles.data]);
  const spans = useMemo(() => order.map(tile => ({ firstHour: Math.max(firstHour, tile * STEP), lastHour: Math.min(lastHour, tile * STEP + STEP - 1) })), [order, firstHour, lastHour]);
  const data = ranking.data && regions.length ? [...replies].reverse().find((reply): reply is AirTimelineReply => reply !== undefined) : undefined;
  const hasData = data !== undefined;
  const heading = following && hasData && !needsViewpoint && regions.length > 0;
  const momentParams = useMemo(() => heading ? airParams(mode) : null, [heading, mode]);
  const head = useHeadProduct(runtime, gate, 'air-superiority', momentParams, { active, validate: validateAirSuperiority, structure: momentRows });
  const moment = momentParams ? head.data : undefined;
  const newest = head.current;
  const lastRead = useMemo(() => data?.days.reduce<number | null>((held, read) => read ?? held, null) ?? null, [data]);
  const sides = useMemo(() => {
    const held: AirSide[] = [];
    for (const side of [...replies.flatMap(reply => reply?.sides ?? NO_SIDES), ...(moment?.sides ?? NO_SIDES)]) if (!held.some(known => known.key === side.key)) held.push(side);
    return held;
  }, [replies, moment]);
  const viewpoint = useMemo(() => {
    const side = data?.viewpoint == null ? null : data.sides[data.viewpoint];
    const at = side ? sides.findIndex(held => held.key === side.key) : -1;
    return at < 0 ? null : at;
  }, [data, sides]);
  const air = useAirLooks(props, data ? { sides, viewpoint } : undefined, lastRead, mode);
  const { looks, tokens } = air;
  const joined = useMemo(() => joinTiles(replies, spans, looks, regions), [replies, spans, looks, regions]);
  const { rows, periods } = joined;
  const labels = useMemo(() => regions.map(region => region.name), [regions]);
  const live = head.live;
  const liveTail = useMemo<Live<HeatTail | null> | undefined>(() => moment && data ? {
    read: () => {
      const at = head.shownAt();
      return at === null ? null : headTail(newest() ?? moment, live.read(), looks, regions, at, data.dayHours).tail;
    },
    subscribe: live.subscribe,
  } : undefined, [moment, data, live, looks, regions, head.shownAt, newest]);
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo<LiveHead | undefined>(() => track && widget.time.mode !== 'pin'
    ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined, [track, widget.time.mode, runtime.clock, axisOffset]);
  const playhead = !track || !window ? null : widget.time.mode === 'pin' ? window.lastHour : Math.floor(runtime.clock.getSnapshot().position + axisOffset);
  const extent: [number, number] = [firstHour, widget.time.mode === 'range' ? lastHour : track?.branch.lastHour ?? lastHour];
  useShownEachFrame(runtime.clock, () => moment ? head.shownAt() : lastRead, head.live);
  useDiagnostics(onDiagnostics, useMemo(() => [...(ranking.data?.warnings ?? []), ...replies.flatMap(reply => reply?.warnings ?? []), ...(moment?.warnings ?? []), ...air.warnings],
    [ranking.data, replies, moment, air.warnings]));
  const menu = useContextMenu('Air superiority timeline');
  const onMenu = useCallback((hour: number, x: number, y: number) => {
    if (track) menu.open(x, y, [jumpItem(runtime, track.id, hour)]);
  }, [track, menu.open, runtime]);
  const viewport = useViewportHeight();
  const tooltip = useCallback((row: HeatRow, period: HeatPeriod | null, hour: number) => {
    if (!data) return null;
    const regionId = Number(row.id);
    const region = { name: row.label };
    const at = moment ? head.shownAt() : null;
    let view;
    if (moment && at !== null && hour >= Math.floor(at / data.dayHours) * data.dayHours) {
      view = headTail(newest() ?? moment, live.read(), looks, regions, at, data.dayHours).views.get(regionId);
    } else {
      const reply = replies.find(item => item && dayOf(item, period?.firstHour ?? hour) !== null);
      const day = reply ? dayOf(reply, period?.firstHour ?? hour) : null;
      const timelineRow = reply?.rows.find(item => item.region === regionId);
      const cell = day === null || !timelineRow ? undefined : cellAt(timelineRow, day);
      view = !reply || day === null || cell === undefined ? undefined : dayView(cell, reply, looks, sideOrder(reply.sides), day);
    }
    if (!view) return null;
    const full = airTooltip(region, view, mode.mode === 'viewpoint', air.country, !!mode.sides.length, air.faction);
    return fitTooltip([airSummaryTooltip(full, view)], viewport, { keep: 1, full });
  }, [data, replies, moment, head.shownAt, live, looks, regions, mode.mode, mode.sides.length, air.country, air.faction, viewport]);
  const legend = useMemo(() => [...air.legend,
    ...(options.look === 'split' ? [] : [{ id: 'contested', label: 'Contested (small lead)', color: mix(tokens.surface, tokens.text, 0.2) }]),
    ...(mode.theatres.length ? [{ id: 'filtered', label: 'Outlined = outside selected theatres' }] : [])],
  [air.legend, mode.theatres.length, options.look, tokens.surface, tokens.text]);
  if (needsViewpoint) return <WidgetState kind="empty" icon="plane" message="Choose a dashboard country or a viewpoint in the widget settings." />;
  if (!track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see its air superiority." />;
  if (!window || beforeBranch) return <WidgetState kind="outside" />;
  const error = ranking.error ?? [...tiles.errors.values()][0] ?? cut.error;
  if (error) return <WidgetState kind="error" failure={error} onRetry={() => { ranking.retry(); tiles.retry(); cut.retry(); }} />;
  if (!ranking.data) return <WidgetState kind="loading" label="Loading air superiority…" />;
  if (!rows.length) return <WidgetState kind="empty" message="No air regions match these filters." />;
  if (!data) return <WidgetState kind="loading" label="Loading air superiority…" />;
  return <div ref={head.root} className="air-timeline">
    <div className="air-pattern-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: tokens.surface }} />No activity</span>
      <span><i aria-hidden="true" style={{ background: `color-mix(in srgb, ${tokens.error} ${tokens.gapOpacity * 100}%, ${tokens.surface})` }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />Unknown</span>
    </div>
    <div className="air-timeline-chart"><HeatChart label="Air superiority timeline" rows={rows} periods={periods} liveTail={liveTail} labelCatalogue={labels}
      extent={extent} playhead={playhead} liveHead={liveHead} branchStart={track.branch.firstHour} legend={legend} afterPlayhead={!following} loading={joined.loading}
      look={options.look === 'split' ? 'split' : 'margin'} active={active} revision={joined} hoursPerSecond={hoursPerSecond} tooltip={tooltip} onContextMenu={onMenu} /></div>
    {menu.menu}
  </div>;
}
