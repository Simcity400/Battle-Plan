import { countryFromKey } from '../../dashboard/countryKey.ts';
import { pageClock } from '../../dashboard/widgetClock.ts';
import { asFailure } from '../../dashboard/queryError.ts';
import { memo, useCallback, useEffect, useMemo,  useRef, useState } from 'react';
import type { NarrowTable } from '../../charts/table/options';
import { narrowTableOf } from '../../charts/table/options.ts';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import ChordChart from '../../charts/chord/ChordChart';
import type { ChordFrame } from '../../charts/chord/ChordChart';
import type { ChordHit } from '../../charts/chord/layout';
import type { ChordData } from '../../charts/chord/layout';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { compactStackExtent, forEachCompactStackSegment, forEachJoinedSegment, stackEdges } from '../../charts/stack';
import { runAt } from '../../charts/series';
import { heldStackSegment } from '../../charts/race/countryRace';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { jumpItem } from '../../charts/jump';
import { catalogueColours } from '../../charts/palette';
import { useCountryColours } from '../../charts/race/countryColours';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt';
import { STYLES } from '../../dashboard/useCountriesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilterQuery, worldRequestTags } from '../../dashboard/groups.ts';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useHeadHour } from '../army-size/headHour.ts';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { viewChange, viewWindows } from '../casualties-breakdown/casualtiesBreakdownData';
import { CasualtyPairsCursor, casualtyPairsAt, casualtyPairTags, casualtyPairWindows, exactCount, pairAmount, validateCasualtyPairs } from './casualtyPairsData.ts';
import type { CasualtyPairsReply, PairFlow } from './casualtyPairsData';
import { CasualtyChordCache, CasualtyChordSegmentCache, chordNameHour, chordNamesAsked, chordPairs, pairTooltip } from './casualtyChord.ts';
import { useIdentityHour, useIdentityKey } from '../../dashboard/atHead.ts';
import type { CasualtyChord } from './casualtyChord';
import { casualtyPairIds, pairDrillAt, pairHistory, pinPairs, readPairSelection } from './casualtyPairDrill';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import { liveAt, mapLive, valuesOf } from '../../charts/live.ts';
import type { Live, LiveValues } from '../../charts/live';
import type { SortValue } from '../../charts/table/logic';
import { definition, DETAIL } from './definition';
import './casualtyPairs.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import {  regionHour, axisAt, recordedHour } from '../../dashboard/regionTime.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const SORT = { column: 'value', direction: 'descending' } as const;
const pairKey = (flow: PairFlow) => flow.id;
const flowOf = (key: string) => key.split('\u0000')[0];
const AMOUNT: TableColumn<PairFlow> = { key: 'value', header: 'Casualties', label: 'Casualties', numeric: true, width: 'max-content',
  value: flow => flow.value, render: pairAmount };

const PairRows = memo(function PairRows({ rows, label, hour, colors, columns, speed, height, onPick, runtime, trackId, narrow, live, totals }: {
  rows: readonly PairFlow[]; label: CountryLabel; hour: number; colors: ReadonlyMap<string, string>;
  columns: readonly TableColumn<PairFlow>[]; speed: number | undefined; height: number; onPick: (hour: number) => void;
  runtime: WidgetProps['runtime']; trackId: string; narrow: NarrowTable;
  live?: Live<LiveValues<SortValue>>; totals?: LiveTexts;
}) {
  const subtotal = rows.some(row => row.incomplete);
  const total = useMemo(() => exactCount(rows.reduce((sum, row) => sum + row.value, 0n)), [rows]);
  const tableColumns = useMemo(() => {
    const positive = rows.reduce((sum, row) => sum + (row.value > 0n ? row.value : 0n), 0n);
    const incomplete = new Set(rows.filter(row => row.incomplete).map(row => row.id));
    return [...columns.map(column => column.key === 'value' ? { ...column,
      render: (flow: PairFlow) => <span><i className="casualty-pair-swatch" style={{ background: colors.get(flow.id) }} aria-hidden="true" />{pairAmount(flow)}</span>,
      paint: (value: NonNullable<SortValue>, key: string) => pairAmount({ value: BigInt(value), incomplete: incomplete.has(flowOf(key)) }),
    } : column), { key: 'share', label: 'Share', header: 'Share', numeric: true, width: 'max-content', priority: 1,
      value: (flow: PairFlow) => subtotal || positive === 0n || flow.value <= 0n ? null : Number(flow.value * 1000n / positive) / 10,
      render: (flow: PairFlow) => subtotal || positive === 0n || flow.value <= 0n ? '—' : `${Number(flow.value * 1000n / positive) / 10}%`,
      paint: (value: NonNullable<SortValue>) => `${value}%`,
    }];
  }, [rows, columns, subtotal, colors]);
  const menu = useCallback((flow: PairFlow) => [{ label: 'Show this day', run: () => onPick(flow.readHour) }, jumpItem(runtime, trackId, flow.readHour)],
    [onPick, runtime, trackId]);
  const tooltip = useCallback((flow: PairFlow) => fitTooltip([pairTooltip([flow], label, hour)], height), [label, hour, height]);
  return <><p className="casualty-breakdown-total">{totals ? <b><LiveText live={totals} at="total" /></b>
    : <>{subtotal ? 'Recorded subtotal' : 'Signed total'}: <b>{total}</b></>}</p>
    <div className="casualty-pair-table"><Table rowRevision={row => [row, tableColumns]} label="Casualty pairs" rows={rows} rowKey={pairKey}
      sort={SORT} hoursPerSecond={speed} cellBars={false} rankMarkers={false} menu={menu} tooltip={tooltip} columns={tableColumns} narrow={narrow} live={live} /></div></>;
});

export default function CasualtyPairsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const drillControl = useDrill(runtime, useRegion(widget), widget.options?.show === DETAIL.id ? DETAIL.id : null);
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const speed = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(definition.options, widget.options);
  const tokens = useChartTokens(), height = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<readonly string[] | null>(null);
  const [chartRange, setChartRange] = useState<readonly [number, number] | null>(null);
  const [chartError, setChartError] = useState<string | null>(null);
  const menu = useContextMenu('Casualty pair actions');
  const exactFrames = useRef(new WeakMap<ChordData, CasualtyChord>());
  const snapshotCursor = useMemo(() => new CasualtyPairsCursor(), []);
  const frameCursor = useMemo(() => new CasualtyPairsCursor(), []);
  const chordCache = useMemo(() => new CasualtyChordCache(), []);
  const segmentCache = useMemo(() => new CasualtyChordSegmentCache(), []);
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'casualty-pairs',
    params: { branchId: track.branch.id, countries: [], between: false, window: null } }) : null;
  const manifest = useProduct<CasualtyPairsReply>(runtime.client, request, active, validateCasualtyPairs, track?.source);
  const drillFollows = drill.breakdown !== null && drillControl.following;
  const headWindow = useCallback((at: number) => String(casualtyPairWindows(manifest.data, at, false)[0] ?? -1), [manifest.data]);
  const drillRows = useRef<() => string>(() => '');
  const playback = useHeadHour(runtime.clock, active && (widget.time.mode === 'follow' || drillFollows),
    at => drillFollows ? `${headWindow(at)}:${drillRows.current()}` : headWindow(at));
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = window?.lastHour ?? track?.branch.lastHour ?? 0;
  const pinned = options.show === DETAIL.id;
  const open = pinned || drill.breakdown !== null;
  const effectiveDrill = drill.breakdown ? { ...drill, follow: drillControl.following } : CLOSED;
  const clockHour = track ? Math.floor(playback) + track.axisOffset : null;
  const drillRegion = drillControl.region;
  const detailAt = open ? regionHour(drillRegion, track, runtime.tracks, axisAt(playback)) : recordedHour(track, hour);
  const detailHour = detailAt ?? hour;
  const outside = !!track && (hour < track.branch.firstHour || hour > track.branch.lastHour);
  const detailOutside = !!track && detailAt === null;
  const pickDay = useCallback((at: number) => dispatch(drill.breakdown ? { type: 'pick', hour: at }
    : { type: 'open', breakdown: DETAIL.id, hour: at, follow: false }), [drill.breakdown]);
  const follow = () => drill.breakdown ? drillControl.follow()
    : dispatch({ type: 'open', breakdown: DETAIL.id, hour: detailHour, follow: drillControl.followsAt(detailHour) });
  const savedSelection = useMemo(() => track ? readPairSelection(options.pairSelection, track) : null, [options.pairSelection, track]);
  const grouped = widget.countries.kind === 'group';
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = defaultCountryKey(track);
  const fallbackTags = useMemo(() => fallback ? [fallback] : [], [fallback]);
  const branchEnd = track?.branch.lastHour ?? 0;
  const filterQuery = useMemo(() => worldFilterQuery(widget.countries, fallbackTags, groups.data, branchEnd),
    [widget.countries, fallbackTags, groups.data, branchEnd]);
  const selection = useMemo(() => worldRequestTags(widget.countries, fallbackTags, groups.data, branchEnd),
    [widget.countries, fallbackTags, groups.data, branchEnd]);
  const currentWindows = casualtyPairWindows(manifest.data, hour, widget.time.mode === 'follow');
  const detailWindows = open ? casualtyPairWindows(manifest.data, detailHour, effectiveDrill.follow) : [];
  const chartWindows = open && manifest.data ? viewWindows(manifest.data.windows, chartRange) : [];
  const onChartView = useCallback((range: readonly [number, number]) => setChartRange(viewChange(manifest.data?.windows ?? [], range)), [manifest.data]);
  const wanted = [...new Set([...currentWindows, ...detailWindows, ...chartWindows])].sort((a, b) => a - b);
  const between = options.between === 'between';
  const identity = useMemo(() => track && selection.ready
    ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, selection.tags, between]) : null, [track, selection, between]);
  const histories = useProductWindows<CasualtyPairsReply>(runtime.client, identity, index => ({ ...request!,
    params: { branchId: track!.branch.id, countries: selection.tags.map(countryFromKey), between, window: index }, asksFrom: manifest.data?.windows[index]?.firstHour }), wanted, active && manifest.data?.available === true,
    validateCasualtyPairs, track?.source, true);
  const data = currentWindows.length ? histories.data.get(currentWindows[0]) : undefined;
  const through = hour;
  const filter = filterQuery.at(through);
  const error = currentWindows.length ? histories.errors.get(currentWindows[0]) : undefined;
  const detailData = detailWindows.length ? histories.data.get(detailWindows[0]) : undefined;
  const detailError = detailWindows.length ? histories.errors.get(detailWindows[0]) : undefined;
  const snapshot = useMemo(() => data ? snapshotCursor.at(data, through, grouped ? filter.tags : null, between)
    : { countries: [], flows: [] }, [snapshotCursor, data, through, filter.tags, grouped, between]);
  const chartReplies = useMemo(() => chartWindows.flatMap(index => histories.data.has(index) ? [histories.data.get(index)!] : []),
    [chartWindows.join(','), histories.data]);
  const selectedIds = useMemo(() => pinned ? savedSelection ?? casualtyPairIds([...histories.data.values()]) : detail ?? [],
    [pinned, savedSelection, histories.data, detail]);
  const selectionKey = JSON.stringify(selectedIds);
  const selected = useMemo(() => new Set(selectedIds), [selectionKey]);
  const detailThrough = detailHour;
  const rows = useMemo(() => detailData ? casualtyPairsAt(detailData, detailThrough).flows.filter(flow => selected.has(flow.id)) : [],
    [detailData, detailThrough, selected]);
  const drillLive = useMemo(() => open && effectiveDrill.follow && track ? liveAt(runtime.clock, position => Math.floor(position) + track.axisOffset, at => {
    const index = casualtyPairWindows(manifest.data, at, false)[0];
    return pairDrillAt(index === undefined ? undefined : histories.data.get(index), at, selected);
  }) : undefined, [open, effectiveDrill.follow, track, runtime.clock, manifest.data, histories.data, selected]);
  drillRows.current = () => drillLive?.read().rows ?? '';
  const drillValues = useMemo(() => drillLive && mapLive(drillLive, made => valuesOf<SortValue>(made.values, key => key.endsWith('share') ? '—' : 'Not recorded')),
    [drillLive]);
  const drillTotals = useMemo<LiveTexts | undefined>(() => drillLive && { text: () => drillLive.read().total, subscribe: drillLive.subscribe }, [drillLive]);
  const drillDay = useMemo<LiveTexts | undefined>(() => drillLive && { text: () => dayLabel(drillLive.hour()), subscribe: drillLive.subscribe }, [drillLive]);
  const following = active && widget.time.mode === 'follow';
  const windowTags = useMemo(() => casualtyPairTags(currentWindows.map(index => histories.data.get(index))),
    [currentWindows.join(','), histories.data]);
  const identityHour = useIdentityHour(runtime.client, track, windowTags, runtime.clock, following,
    clockHour => track ? trackHourOf(widget, track, runtime.tracks, clockHour) : null);
  const identitySince = useIdentityKey(runtime.client, track, following);
  const named = chordNamesAsked(following, identityHour, through, windowTags, snapshot.countries);
  const facts = useMemo(() => [...named.tags.map(tag => ({ tag, hour: named.hour })),
    ...rows.flatMap(flow => [{ tag: flow.from, hour: detailThrough }, { tag: flow.to, hour: detailThrough }])], [named.tags, named.hour, rows, detailThrough]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, facts, active);
  const styles = useCountryFactsAt(runtime.client, track, facts, active, STYLES);
  const colorRevision = useMemo(() => ({}), [styles]);
  const color = useCountryColours(named.tags, tag => { const style = styles.get(nameKey(named.hour, tag)); return style instanceof Error ? null : style?.color; }, tokens, colorRevision);
  const mutedColor = useMemo(() => (_tag: string) => tokens.muted, [tokens.muted]);
  const detailLabel = useMemo(() => labelAt(detailThrough), [labelAt, detailThrough]);
  const frameAt = useMemo(() => {
    let lastHour = NaN, lastSeek = -1, frame: ChordFrame = { data: null };
    let labels: { hour: number; at: CountryLabel } | null = null;
    return (): ChordFrame => {
      const clock = runtime.clock.getSnapshot();
      const seek = widget.time.mode === 'follow' ? clock.seekSequence : 0;
      const shown = track ? resolveWindow(widget, track, runtime.tracks, pageClock(runtime.clock).getHour())?.lastHour ?? NaN : NaN;
      const playing = widget.time.mode === 'follow' && clock.playing;
      if (shown === lastHour && seek === lastSeek) return { ...frame, playing };
      const index = casualtyPairWindows(manifest.data, shown, false)[0];
      const reply = index === undefined ? undefined : histories.data.get(index);
      const read = shown;
      const span = reply?.observed.find(span => span.firstHour <= read && read <= span.lastHour);
      const selected = filterQuery.at(read);
      const at = reply && span && (!grouped || groups.data && !selected.reason)
        ? frameCursor.at(reply, read, grouped ? selected.tags : null, between) : null;
      const since = at && following ? identitySince(read, named.tags) : null;
      const nameHour = chordNameHour(read, since === null ? null : Number(since));
      if (at && labels?.hour !== nameHour) labels = { hour: nameHour, at: labelAt(nameHour) };
      const exact = at && labels ? chordCache.at(at, Number(options.mostCountries), read, labels.at,
        nameHour === named.hour ? color : mutedColor, tokens.muted) : null;
      if (exact) exactFrames.current.set(exact.data, exact);
      frame = { data: exact?.data ?? null, revision: exact, playing,
        segment: segmentCache.at(at, identity, span?.firstHour, seek) };
      lastHour = shown; lastSeek = seek;
      return frame;
    };
  }, [runtime.clock, runtime.tracks, track, widget, manifest.data, histories.data, groups.data, grouped, filterQuery,
    options.mostCountries, labelAt, color, mutedColor, following, identitySince, named.tags, named.hour, tokens.muted, identity, between, frameCursor,
    chordCache, segmentCache]);
  const chordTooltip = useCallback((hit: ChordHit, frame: ChordData) => {
    const exact = exactFrames.current.get(frame);
    return exact ? fitTooltip([pairTooltip(chordPairs(hit, exact), exact.labels, exact.hour)], height) : null;
  }, [height]);
  const stack = useMemo(() => pairHistory(chartReplies, selectedIds), [chartReplies, selected]);
  const allPairIds = useMemo(() => [...new Set([...casualtyPairIds([...histories.data.values()]), ...selectedIds])], [histories.data, selectedIds]);
  const colorById = useMemo(() => catalogueColours(allPairIds, tokens.surface), [allPairIds, tokens.surface]);
  const colors = useMemo(() => selectedIds.map(id => colorById.get(id) ?? tokens.muted), [selectedIds, colorById, tokens.muted]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, theme, state) => {
      const until = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      forEachJoinedSegment(piece => forEachCompactStackSegment(stack, frame.x[0], until, (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth) / 2,
        segment => piece(heldStackSegment(segment)), at => stackEdges(stack.bands.map(runs => runAt(runs, at)?.value ?? 0))),
      held => drawBands(ctx, frame, theme, held.hours, held.bands, colors, held));
    }, extent: range => compactStackExtent(stack, range[0], range[1]),
  }], [stack, colors]);
  const pickOwner = useMemo(() => ({ client: runtime.client, identity, selected, stack }), [runtime.client, identity, selected, stack]);
  const openPairs = (ids: readonly string[], at: number) => {
    if (!active) return;
    setDetail(ids);
    dispatch({ type: 'open', breakdown: DETAIL.id, hour: at, follow: drillControl.followsAt(at) });
  };
  const columns = useMemo<TableColumn<PairFlow>[]>(() => [
    { key: 'from', header: 'Dealt by', label: 'Dealt by', value: flow => detailLabel(flow.from).name,
      render: flow => <CountryCell {...detailLabel(flow.from)} /> },
    { key: 'to', header: 'Taken by', label: 'Taken by', value: flow => detailLabel(flow.to).name,
      render: flow => <CountryCell {...detailLabel(flow.to)} /> },
    AMOUNT,
  ], [detailLabel]);
  useEffect(() => { dispatch({ type: 'close' }); setDetail(null); setChartRange(null); }, [runtime.client, identity, widget.time, options.show, options.pairSelection]);
  const diagnostics = useMemo(() => [...new Set([...(manifest.data?.warnings ?? []),
    ...[...histories.data.values()].flatMap(reply => reply.warnings), ...(groups.data?.warnings ?? []), ...[...styles.values()].filter((value): value is Error => value instanceof Error).map(error => error.message)])], [manifest.data, histories.data, groups.data, styles]);
  useDiagnostics(onDiagnostics, diagnostics);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see casualties between countries." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (manifest.error) return <WidgetState kind="error" failure={manifest.error} onRetry={manifest.retry} />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (manifest.data?.available === false) return <>{groupFailure}<WidgetState kind="unavailable" message="Not in this recording" /></>;
  if (selection.reason) return <>{groupFailure}<WidgetState kind="empty" message={selection.reason} /></>;
  if (!manifest.data || needsGroups && !groups.data) return <>{groupFailure}<WidgetState kind="loading" label="Loading casualty pairs…" /></>;
  if (pinned && options.pairSelection && !savedSelection) return <>{groupFailure}<WidgetState kind="unavailable" message="Saved pairs unavailable. Select them again." /></>;
  const body = <div className="casualty-pair-details">
    {pinned && <p className="chart-line-key">{drillDay ? <LiveText live={drillDay} at="day" /> : dayLabel(detailHour)} {!effectiveDrill.follow
      && <button type="button" className="button small" onClick={follow}>Follow playback</button>}</p>}
    {detailOutside ? <WidgetState kind="outside" />
      : detailError ? <WidgetState kind="error" failure={detailError} onRetry={histories.retry} />
      : !detailData ? <WidgetState kind="loading" label="Loading this moment…" />
        : !detailData.observed.some(span => span.firstHour <= detailThrough && detailThrough <= span.lastHour)
          ? <p className="drill-empty">Casualty pairs were not recorded at {dayLabel(detailHour)}.</p>
          : !rows.length ? <p className="drill-empty">No selected casualty pairs at this time.</p>
            : <PairRows rows={rows} label={detailLabel} hour={detailThrough} colors={colorById} columns={columns} speed={speed} narrow={narrowTableOf(options)}
              height={height} onPick={pickDay} runtime={runtime} trackId={track.id} live={drillValues} totals={drillTotals} />}
    <div className="casualty-pair-history"><TimeChart label="Positive recorded casualties by directed pair over time"
      extent={[track.branch.firstHour, track.branch.lastHour]} branchStart={track.branch.firstHour}
      playhead={effectiveDrill.follow ? clockHour : null} playback="cursor" hoursPerSecond={speed}
      axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers}
      keyHours={stack.hours} active={active && open} revision={stack} pickOwner={pickOwner} onViewChange={onChartView}
      onPick={pickDay} overlay={(ctx, frame, theme) => drawPlayhead(ctx, frame, theme, detailHour)}
      onContextMenu={(at, x, y) => menu.open(x, y, [{ label: 'Show this day', run: () => pickDay(at) }, jumpItem(runtime, track.id, Math.floor(at))])}
      tooltip={at => {
        const index = casualtyPairWindows(manifest.data, at, false)[0];
        const reply = index === undefined ? undefined : histories.data.get(index);
        const flows = reply ? casualtyPairsAt(reply, at).flows.filter(flow => selected.has(flow.id)) : [];
        return { title: dayLabel(Math.floor(at)), rows: [{ label: flows.some(flow => flow.incomplete) ? 'Recorded subtotal' : 'Signed total',
          value: reply?.observed.some(span => span.firstHour <= at && at <= span.lastHour)
            ? flows.length ? exactCount(flows.reduce((sum, flow) => sum + flow.value, 0n)) : 'No selected casualty pairs at this time.'
            : reply ? 'Not recorded' : 'Loading…' }] };
      }} /></div>
    {chartWindows.some(index => !histories.data.has(index) && !histories.errors.has(index)) && <p className="drill-empty">Loading history in the visible time range…</p>}
    {chartWindows.some(index => histories.errors.has(index)) && <button type="button" className="button small" onClick={histories.retry}>Retry history</button>}
  </div>;
  const main = outside ? <WidgetState kind="outside" />
    : error ? <WidgetState kind="error" failure={error} onRetry={histories.retry} />
    : !data ? <WidgetState kind="loading" label="Loading casualty pairs…" />
      : filter.reason ? <WidgetState kind="empty" message={filter.reason} />
        : !data.observed.some(span => span.firstHour <= through && through <= span.lastHour)
          ? <WidgetState kind="unavailable" message="Casualty pairs were not recorded at this moment." />
          : !snapshot.flows.length ? <WidgetState kind="empty" message="No recorded country pairs at this moment." /> : <>
    <div className="casualty-pairs-chart"><ChordChart label="Casualties between countries" caption="Ribbon colour = the country dealing casualties"
      frame={{ data: null }} frameAt={frameAt}
      hoursPerSecond={speed} active={active && (!open || options.drill === 'popout') && !pinned} onError={setChartError}
      tooltip={chordTooltip}
      onPick={(hit, frame) => { const exact = exactFrames.current.get(frame);
        if (exact) openPairs(chordPairs(hit, exact).map(flow => flow.id), exact.hour); }}
      onContextMenu={(hit, frame, x, y) => {
        const exact = exactFrames.current.get(frame);
        const flows = exact ? chordPairs(hit, exact) : [];
        if (flows.length) menu.open(x, y, [{ label: 'Recorded pairs', icon: 'grid', run: () => openPairs(flows.map(flow => flow.id), exact!.hour) },
          jumpItem(runtime, track.id, exact!.hour)]);
      }} />
      {!snapshot.flows.some(flow => flow.value > 0n) && <WidgetState kind="empty" message="No positive attributable casualties are recorded at this moment." />}</div>
    {chartError && <WidgetState kind="error" failure={asFailure(chartError)} />}
    <button type="button" className="button small" onClick={() => openPairs(snapshot.flows.map(flow => flow.id), through)}>All pairs</button>
    {data.hasGap && <p className="chart-line-key">Some losses may be missing: the casualty tables were not written at some hours of this period.</p>}
    {snapshot.flows.some(flow => flow.value < 0n) && <button type="button" className="button small"
      onClick={() => openPairs(snapshot.flows.filter(flow => flow.value < 0n).map(flow => flow.id), through)}>Corrections</button>}
  </>;
  return <div className="casualty-pairs" ref={anchor}>{groupFailure}
    {pinned ? body : <DrillHost drill={drillControl} runtime={runtime} subject={open} mode={options.drill === 'popout' ? 'popout' : 'inline'} title="Casualty pairs"
      day={drillDay ? <LiveText live={drillDay} at="day" /> : dayLabel(detailHour)} canFollow
      pinRefused={drillControl.pinRefused} onPin={selectedIds.length && drillControl.time() ? () => { const time = drillControl.time(); if (time) runtime.addWidgetBelow(widget.id, pinPairs(widget, track, selectedIds, time)); } : undefined}
      onClose={() => { dispatch({ type: 'close' }); setDetail(null); }}
      returnFocus=".chord-chart canvas" anchor={anchor} main={main}>{body}</DrillHost>}
    {menu.menu}
  </div>;
}
