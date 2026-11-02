import { asFailure } from '../../dashboard/queryError.ts';
import { memo, useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { NarrowTable } from '../../charts/table/options';
import { narrowTableOf } from '../../charts/table/options.ts';
import WidgetState from '../../components/WidgetState';
import { useParityRows } from '../../dashboard/parityRows';
import { useContextMenu } from '../../components/ContextMenu';
import ChordChart from '../../charts/chord/ChordChart';
import type { ChordFrame } from '../../charts/chord/ChordChart';
import type { ChordHit } from '../../charts/chord/layout';
import type { ChordData } from '../../charts/chord/layout';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { compactStackExtent, forEachCompactStackSegment, stackEdges } from '../../charts/stack';
import { runAt } from '../../charts/series';
import { heldStackSegment } from '../../charts/race/countryRace';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { jumpItem } from '../../charts/jump';
import { useCountryColours } from '../../charts/race/countryColours';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt';
import { STYLES } from '../../dashboard/useCountriesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING, worldFilterQuery, worldRequestTags } from '../../dashboard/groups.ts';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { preparedPrefix, preparedThrough } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { viewChange, viewWindows } from '../casualties-breakdown/casualtiesBreakdownData';
import { CasualtyPairsCursor, casualtyPairsAt, casualtyPairWindows, exactCount, pairAmount, validateCasualtyPairs } from './casualtyPairsData.ts';
import type { CasualtyPairsReply, PairFlow } from './casualtyPairsData';
import { CasualtyChordCache, CasualtyChordSegmentCache, chordPairs, pairTooltip } from './casualtyChord.ts';
import type { CasualtyChord } from './casualtyChord';
import { casualtyPairIds, pairHistory, pinPairs, readPairSelection } from './casualtyPairDrill';
import { definition, DETAIL } from './definition';
import './casualtyPairs.css';

const SORT = { column: 'value', direction: 'descending' } as const;
const pairKey = (flow: PairFlow) => flow.id;
// Columns that name no country keep their accessors: arriving labels never re-sort by them.
const AMOUNT: TableColumn<PairFlow> = { key: 'value', header: 'Casualties', label: 'Casualties', numeric: true, width: 'max-content',
  value: flow => flow.value, render: pairAmount };

/** The selected pairs' exact signed rows. Built only once the breakdown mounts,
 * and again only when the rows, their labels or colours change. */
const PairRows = memo(function PairRows({ rows, label, hour, colors, columns, speed, height, onPick, runtime, trackId, narrow }: {
  rows: readonly PairFlow[]; label: CountryLabel; hour: number; colors: ReadonlyMap<string, string>;
  columns: readonly TableColumn<PairFlow>[]; speed: number | undefined; height: number; onPick: (hour: number) => void;
  runtime: WidgetProps['runtime']; trackId: string; narrow: NarrowTable;
}) {
  const subtotal = rows.some(row => row.incomplete);
  const total = useMemo(() => exactCount(rows.reduce((sum, row) => sum + row.value, 0n)), [rows]);
  const tableColumns = useMemo(() => {
    const positive = rows.reduce((sum, row) => sum + (row.value > 0n ? row.value : 0n), 0n);
    return [...columns.map(column => column.key === 'value' ? { ...column,
      render: (flow: PairFlow) => <span><i className="casualty-pair-swatch" style={{ background: colors.get(flow.id) }} aria-hidden="true" />{pairAmount(flow)}</span>,
    } : column), { key: 'share', label: 'Share', header: 'Share', numeric: true, width: 'max-content', priority: 1,
      value: (flow: PairFlow) => subtotal || positive === 0n || flow.value <= 0n ? null : Number(flow.value * 1000n / positive) / 10,
      render: (flow: PairFlow) => subtotal || positive === 0n || flow.value <= 0n ? '—' : `${Number(flow.value * 1000n / positive) / 10}%`,
    }];
  }, [rows, columns, subtotal, colors]);
  const menu = useCallback((flow: PairFlow) => [{ label: 'Show this day', run: () => onPick(flow.readHour) }, jumpItem(runtime, trackId, flow.readHour)],
    [onPick, runtime, trackId]);
  const tooltip = useCallback((flow: PairFlow) => fitTooltip([pairTooltip([flow], label, hour)], height), [label, hour, height]);
  return <><p className="casualty-breakdown-total">{subtotal ? 'Recorded subtotal' : 'Signed total'}: <b>{total}</b></p>
    <div className="casualty-pair-table"><Table rowRevision={row => [row, tableColumns]} label="Casualty pairs" rows={rows} rowKey={pairKey}
      sort={SORT} hoursPerSecond={speed} cellBars={false} rankMarkers={false} menu={menu} tooltip={tooltip} columns={tableColumns} narrow={narrow} /></div></>;
});

export default function CasualtyPairsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const playback = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
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
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = window?.lastHour ?? track?.branch.lastHour ?? 0;
  const pinned = options.show === DETAIL.id;
  const open = pinned || drill.breakdown !== null;
  const effectiveDrill = drill.breakdown ? drill : pinned
    ? { breakdown: DETAIL.id, follow: widget.time.mode === 'follow', hour } : CLOSED;
  const clockHour = track ? Math.floor(playback) + track.axisOffset : null;
  const detailHour = (open ? drillHour(effectiveDrill, clockHour) : hour) ?? hour;
  const outside = !!track && (hour < track.branch.firstHour || hour > track.branch.lastHour);
  const detailOutside = !!track && (detailHour < track.branch.firstHour || detailHour > track.branch.lastHour);
  const pickDay = useCallback((at: number) => dispatch(drill.breakdown ? { type: 'pick', hour: at }
    : { type: 'open', breakdown: DETAIL.id, hour: at, playing: false, playhead: null }), [drill.breakdown]);
  const follow = () => dispatch(drill.breakdown ? { type: 'follow' }
    : { type: 'open', breakdown: DETAIL.id, hour: detailHour, playing: true, playhead: clockHour });
  const savedSelection = useMemo(() => track ? readPairSelection(options.pairSelection, track) : null, [options.pairSelection, track]);
  const grouped = widget.countries.kind === 'group';
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = defaultCountryTag(track);
  const fallbackTags = useMemo(() => fallback ? [fallback] : [], [fallback]);
  const branchEnd = track?.branch.lastHour ?? 0;
  const filterQuery = useMemo(() => worldFilterQuery(widget.countries, fallbackTags, groups.data, branchEnd),
    [widget.countries, fallbackTags, groups.data, branchEnd]);
  const selection = useMemo(() => worldRequestTags(widget.countries, fallbackTags, groups.data, branchEnd),
    [widget.countries, fallbackTags, groups.data, branchEnd]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'casualty-pairs',
    params: { branchId: track.branch.id, tags: [], between: false, window: null } } : null;
  const manifest = useProduct<CasualtyPairsReply>(runtime.client, request, active, validateCasualtyPairs, track?.source);
  const currentWindows = casualtyPairWindows(manifest.data, hour, widget.time.mode === 'follow');
  const detailWindows = open ? casualtyPairWindows(manifest.data, detailHour, effectiveDrill.follow) : [];
  const chartWindows = open && manifest.data ? viewWindows(manifest.data.windows, chartRange) : [];
  const onChartView = useCallback((range: readonly [number, number]) => setChartRange(viewChange(manifest.data?.windows ?? [], range)), [manifest.data]);
  const wanted = [...new Set([...currentWindows, ...detailWindows, ...chartWindows])].sort((a, b) => a - b);
  const between = options.between === 'between';
  const identity = useMemo(() => track && selection.ready
    ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, selection.tags, between]) : null, [track, selection, between]);
  const histories = useProductWindows<CasualtyPairsReply>(runtime.client, identity, index => ({ ...request!,
    params: { branchId: track!.branch.id, tags: selection.tags, between, window: index } }), wanted, active && manifest.data?.available === true,
    validateCasualtyPairs, track?.source, true);
  const data = currentWindows.length ? histories.data.get(currentWindows[0]) : undefined;
  // A window preparation has not reached answers from the prepared prefix's
  // newest window: the flows at the prefix's last hour.
  const through = data ? preparedThrough([data], hour) : hour;
  // Membership at the moment the flows show, not the hour asked for.
  const filter = filterQuery.at(through);
  const error = currentWindows.length ? histories.errors.get(currentWindows[0]) : undefined;
  const detailData = detailWindows.length ? histories.data.get(detailWindows[0]) : undefined;
  const detailError = detailWindows.length ? histories.errors.get(detailWindows[0]) : undefined;
  const snapshot = useMemo(() => data ? snapshotCursor.at(data, through, grouped ? filter.tags : null, between)
    : { countries: [], flows: [] }, [snapshotCursor, data, through, filter.tags, grouped, between]);
  useParityRows(anchor, () => ({ kind: 'windowed', label: 'Casualty pairs exact values', complete: !!data,
    total: snapshot.flows.length, rows: snapshot.flows.map(flow => ({ ...flow, value: flow.value.toString() })),
    countries: snapshot.countries, window: data?.window, observed: data?.observed, warnings: data?.warnings }));
  const chartReplies = useMemo(() => chartWindows.flatMap(index => histories.data.has(index) ? [histories.data.get(index)!] : []),
    [chartWindows.join(','), histories.data]); // eslint-disable-line react-hooks/exhaustive-deps
  const selectedIds = useMemo(() => pinned ? savedSelection ?? casualtyPairIds([...histories.data.values()]) : detail ?? [],
    [pinned, savedSelection, histories.data, detail]);
  const selectionKey = JSON.stringify(selectedIds);
  const selected = useMemo(() => new Set(selectedIds), [selectionKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const detailThrough = detailData ? preparedThrough([detailData], detailHour) : detailHour;
  const rows = useMemo(() => detailData ? casualtyPairsAt(detailData, detailThrough).flows.filter(flow => selected.has(flow.id)) : [],
    [detailData, detailThrough, selected]);
  const facts = useMemo(() => [...snapshot.countries.map(tag => ({ tag, hour: through })),
    ...rows.flatMap(flow => [{ tag: flow.from, hour: detailThrough }, { tag: flow.to, hour: detailThrough }])], [snapshot.countries, through, rows, detailThrough]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, facts, active);
  const styles = useCountryFactsAt(runtime.client, track, facts, active, STYLES);
  const colorRevision = useMemo(() => ({}), [styles]);
  const color = useCountryColours(snapshot.countries, tag => { const style = styles.get(nameKey(through, tag)); return style instanceof Error ? null : style?.color; }, tokens, colorRevision);
  const mutedColor = useMemo(() => (_tag: string) => tokens.muted, [tokens.muted]);
  const detailLabel = useMemo(() => labelAt(detailThrough), [labelAt, detailThrough]);
  // Resolve the clock locally between React ticks. Prefetched windows carry only
  // recorded values; a gap or an unloaded window clears the graph immediately.
  const frameAt = useMemo(() => {
    let lastHour = NaN, lastSeek = -1, frame: ChordFrame = { data: null };
    return (now: number): ChordFrame => {
      const clock = runtime.clock.getSnapshot();
      const seek = widget.time.mode === 'follow' ? clock.seekSequence : 0;
      const shown = track ? resolveWindow(widget, track, runtime.tracks, widget.time.mode === 'follow'
        ? runtime.clock.positionAt(now) : clock.position)?.lastHour ?? NaN : NaN;
      const playing = widget.time.mode === 'follow' && clock.playing;
      if (shown === lastHour && seek === lastSeek) return { ...frame, playing };
      const index = casualtyPairWindows(manifest.data, shown, false)[0];
      const reply = index === undefined ? undefined : histories.data.get(index);
      // A prefix reply's flows hold at its last hour.
      const read = reply ? preparedThrough([reply], shown) : shown;
      const span = reply?.observed.find(span => span.firstHour <= read && read <= span.lastHour);
      const selected = filterQuery.at(read);
      const at = reply && span && (!grouped || groups.data && !selected.reason)
        ? frameCursor.at(reply, read, grouped ? selected.tags : null, between) : null;
      const exact = at ? chordCache.at(at, Number(options.mostCountries), read, labelAt(read),
        shown === hour ? color : mutedColor, tokens.muted) : null;
      if (exact) exactFrames.current.set(exact.data, exact);
      frame = { data: exact?.data ?? null, revision: exact, playing,
        segment: segmentCache.at(at, identity, span?.firstHour, seek) };
      lastHour = shown; lastSeek = seek;
      return frame;
    };
  }, [runtime.clock, runtime.tracks, track, widget, manifest.data, histories.data, groups.data, grouped, filterQuery,
    options.mostCountries, labelAt, color, mutedColor, hour, tokens.muted, identity, between, frameCursor, chordCache, segmentCache]);
  const chordTooltip = useCallback((hit: ChordHit, frame: ChordData) => {
    const exact = exactFrames.current.get(frame);
    return exact ? fitTooltip([pairTooltip(chordPairs(hit, exact), exact.labels, exact.hour)], height) : null;
  }, [height]);
  const stack = useMemo(() => pairHistory(chartReplies, selectedIds), [chartReplies, selected]); // eslint-disable-line react-hooks/exhaustive-deps
  const colors = useMemo(() => selectedIds.map((_id, index) => tokens.categories[index % tokens.categories.length]), [selectedIds, tokens.categories]);
  const colorById = useMemo(() => new Map(selectedIds.map((id, index) => [id, colors[index]])), [selectedIds, colors]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, theme, state) => {
      const until = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      forEachCompactStackSegment(stack, frame.x[0], until, (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth) / 2, piece => {
        const held = heldStackSegment(piece);
        drawBands(ctx, frame, theme, held.hours, held.bands, colors, held);
      }, at => stackEdges(stack.bands.map(runs => runAt(runs, at)?.value ?? 0)));
    }, extent: range => compactStackExtent(stack, range[0], range[1]),
  }], [stack, colors]);
  const pickOwner = useMemo(() => ({ client: runtime.client, identity, selected, stack }), [runtime.client, identity, selected, stack]);
  const openPairs = (ids: readonly string[], at: number) => {
    if (!active) return;
    setDetail(ids);
    dispatch({ type: 'open', breakdown: DETAIL.id, hour: at, playing: widget.time.mode === 'follow' && runtime.clock.getSnapshot().playing,
      playhead: widget.time.mode === 'follow' ? clockHour : null });
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
  // No pair met yet while preparation runs: later blocks may hold some.
  if (manifest.data?.available === false) return <>{groupFailure}{preparedPrefix(manifest.data)?.pending.length
    ? <WidgetState kind="loading" label="This range is still being prepared." />
    : <WidgetState kind="unavailable" message="Not in this recording" />}</>;
  if (selection.reason) return <>{groupFailure}<WidgetState kind="empty" message={selection.reason} /></>;
  if (!manifest.data || needsGroups && !groups.data) return <>{groupFailure}<WidgetState kind="loading" label="Loading casualty pairs…" /></>;
  if (pinned && options.pairSelection && !savedSelection) return <>{groupFailure}<WidgetState kind="unavailable" message="Saved pairs unavailable. Select them again." /></>;
  const body = <div className="casualty-pair-details">
    {detailThrough < detailHour && <p className="chart-line-key" role="status">Casualties through {dayLabel(detailThrough)}; the rest is still being prepared.</p>}
    {pinned && <p className="chart-line-key">{dayLabel(detailHour)} {!effectiveDrill.follow
      && <button type="button" className="button small" onClick={follow}>Follow playback</button>}</p>}
    {detailOutside ? <WidgetState kind="unavailable" message="This time is outside the recording." />
      : detailError ? <WidgetState kind="error" failure={detailError} onRetry={histories.retry} />
      : !detailData ? <WidgetState kind="loading" label="Loading this moment…" />
        : !detailData.observed.some(span => span.firstHour <= detailThrough && detailThrough <= span.lastHour)
          ? <p className="drill-empty">Casualty pairs were not recorded at {dayLabel(detailHour)}.</p>
          : !rows.length ? <p className="drill-empty">No selected casualty pairs at this time.</p>
            : <PairRows rows={rows} label={detailLabel} hour={detailThrough} colors={colorById} columns={columns} speed={speed} narrow={narrowTableOf(options)}
              height={height} onPick={pickDay} runtime={runtime} trackId={track.id} />}
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
  const main = outside ? <WidgetState kind="unavailable" message="This time is outside the recording." />
    : error ? <WidgetState kind="error" failure={error} onRetry={histories.retry} />
    : !data ? <WidgetState kind="loading" label="Loading casualty pairs…" />
      : filter.reason ? <WidgetState kind="empty" message={filter.reason} />
        : !data.observed.some(span => span.firstHour <= through && through <= span.lastHour)
          ? <WidgetState kind="unavailable" message="Casualty pairs were not recorded at this moment." />
          : !snapshot.flows.length ? grouped && !filter.known ? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />
            : <WidgetState kind="empty" message="No recorded country pairs at this moment." /> : <>
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
    {through < hour && <p className="chart-line-key" role="status">Casualties through {dayLabel(through)}; the rest is still being prepared.</p>}
    <button type="button" className="button small" onClick={() => openPairs(snapshot.flows.map(flow => flow.id), through)}>All pairs</button>
    {snapshot.flows.some(flow => flow.value < 0n) && <button type="button" className="button small"
      onClick={() => openPairs(snapshot.flows.filter(flow => flow.value < 0n).map(flow => flow.id), through)}>Corrections</button>}
  </>;
  return <div className="casualty-pairs" ref={anchor}>{groupFailure}
    {pinned ? body : <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={open} title="Casualty pairs"
      day={dayLabel(detailHour)} following={effectiveDrill.follow} canFollow onFollow={follow}
      onPin={selectedIds.length ? () => runtime.addWidgetBelow(widget.id, pinPairs(widget, track, selectedIds, detailHour, effectiveDrill.follow)) : undefined}
      onClose={() => { dispatch({ type: 'close' }); setDetail(null); }}
      returnFocus=".chord-chart canvas" anchor={anchor} main={main}>{body}</DrillHost>}
    {menu.menu}
  </div>;
}
