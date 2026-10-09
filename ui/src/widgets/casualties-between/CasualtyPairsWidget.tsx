import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import ChordChart from '../../charts/chord/ChordChart';
import type { ChordFrame } from '../../charts/chord/ChordChart';
import type { ChordData } from '../../charts/chord/layout';
import DrillHost from '../../charts/drill/DrillHost';
import DrillParts from '../../charts/drill/DrillParts';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { drawBands, drawPlayhead } from '../../charts/draw';
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
import { formatDateTime } from '../../dashboard/calendar';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt';
import { STYLES } from '../../dashboard/useCountriesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilter, worldRequestTags } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { CasualtyPairsReply } from '../../generated/dashboard';
import { casualtyPairsAt, casualtyPairWindows, pairAmount, validateCasualtyPairs } from './casualtyPairsData';
import type { PairFlow } from './casualtyPairsData';
import { casualtyChord, chordPairs, pairTooltip } from './casualtyChord';
import type { CasualtyChord } from './casualtyChord';
import { casualtyPairIds, pairHistory, pinPairs, readPairSelection } from './casualtyPairDrill';
import { definition, DETAIL } from './definition';
import './casualtyPairs.css';

const NO_WARNINGS: string[] = [];
export default function CasualtyPairsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const playback = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const speed = usePlaybackSpeed(runtime.clock);
  const options = resolveOptions(definition.options, widget.options);
  const tokens = useChartTokens(), height = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<readonly string[] | null>(null);
  const [chartRange, setChartRange] = useState<readonly [number, number] | null>(null);
  const [chartError, setChartError] = useState<string | null>(null);
  const menu = useContextMenu('Casualty pair actions');
  const exactFrames = useRef(new WeakMap<ChordData, CasualtyChord>());
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = window?.lastHour ?? track?.branch.lastHour ?? 0;
  const pinned = options.show === DETAIL.id;
  const open = pinned || drill.breakdown !== null;
  const effectiveDrill = drill.breakdown ? drill : pinned
    ? { breakdown: DETAIL.id, follow: widget.time.mode === 'follow', hour } : CLOSED;
  const clockHour = track ? Math.floor(runtime.clock.getHour()) + track.axisOffset : null;
  const detailHour = (open ? drillHour(effectiveDrill, clockHour) : hour) ?? hour;
  const outside = !!track && (hour < track.branch.firstHour || hour > track.branch.lastHour);
  const detailOutside = !!track && (detailHour < track.branch.firstHour || detailHour > track.branch.lastHour);
  const pickDay = (at: number) => dispatch(drill.breakdown ? { type: 'pick', hour: at }
    : { type: 'open', breakdown: DETAIL.id, hour: at, playing: false, playhead: null });
  const follow = () => dispatch(drill.breakdown ? { type: 'follow' }
    : { type: 'open', breakdown: DETAIL.id, hour: detailHour, playing: true, playhead: clockHour });
  const savedSelection = useMemo(() => track ? readPairSelection(options.pairSelection, track) : null, [options.pairSelection, track]);
  const grouped = widget.countries.kind === 'group';
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = defaultCountryTag(track);
  const selection = worldRequestTags(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? hour);
  const filter = worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, hour, track?.branch.lastHour ?? hour);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'casualty-pairs',
    params: { branchId: track.branch.id, tags: [], between: false, window: null } } : null;
  const manifest = useProduct<CasualtyPairsReply>(runtime.client, request, active, validateCasualtyPairs);
  const currentWindows = casualtyPairWindows(manifest.data, hour, widget.time.mode === 'follow');
  const detailWindows = open ? casualtyPairWindows(manifest.data, detailHour, effectiveDrill.follow) : [];
  const chartWindows = open ? manifest.data?.windows.flatMap((span, index) => !chartRange
    || span.firstHour <= chartRange[1] && chartRange[0] <= span.lastHour ? [index] : []) ?? [] : [];
  const wanted = [...new Set([...currentWindows, ...detailWindows, ...chartWindows])].sort((a, b) => a - b);
  const between = options.between === 'between';
  const identity = track && selection.ready ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, selection.tags, between]) : null;
  const histories = useProductWindows<CasualtyPairsReply>(runtime.client, identity, index => ({ ...request!,
    params: { branchId: track!.branch.id, tags: selection.tags, between, window: index } }), wanted, active && manifest.data?.available === true,
    validateCasualtyPairs);
  const data = currentWindows.length ? histories.data.get(currentWindows[0]) : undefined;
  const error = currentWindows.length ? histories.errors.get(currentWindows[0]) : undefined;
  const detailData = detailWindows.length ? histories.data.get(detailWindows[0]) : undefined;
  const detailError = detailWindows.length ? histories.errors.get(detailWindows[0]) : undefined;
  const snapshot = useMemo(() => data ? casualtyPairsAt(data, hour, tag => !grouped || filter.tags === null || filter.tags.has(tag), between)
    : { countries: [], flows: [] }, [data, hour, filter.tags, grouped, between]);
  const chartReplies = useMemo(() => chartWindows.flatMap(index => histories.data.has(index) ? [histories.data.get(index)!] : []),
    [chartWindows.join(','), histories.data]); // eslint-disable-line react-hooks/exhaustive-deps
  const selectedIds = useMemo(() => pinned ? savedSelection ?? casualtyPairIds([...histories.data.values()]) : detail ?? [],
    [pinned, savedSelection, histories.data, detail]);
  const selectionKey = JSON.stringify(selectedIds);
  const selected = useMemo(() => new Set(selectedIds), [selectionKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const rows = useMemo(() => detailData ? casualtyPairsAt(detailData, detailHour).flows.filter(flow => selected.has(flow.id)) : [],
    [detailData, detailHour, selected]);
  const facts = useMemo(() => [...snapshot.countries.map(tag => ({ tag, hour })),
    ...rows.flatMap(flow => [{ tag: flow.from, hour: detailHour }, { tag: flow.to, hour: detailHour }])], [snapshot.countries, hour, rows, detailHour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, facts, active);
  const styles = useCountryFactsAt(runtime.client, track, facts, active, STYLES);
  const colorRevision = useMemo(() => ({}), [styles]);
  const color = useCountryColours(snapshot.countries, tag => styles.get(nameKey(hour, tag))?.color, tokens, colorRevision);
  const detailLabel = useMemo(() => labelAt(detailHour), [labelAt, detailHour]);
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
      const span = reply?.observed.find(span => span.firstHour <= shown && shown <= span.lastHour);
      const selected = worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, shown, track?.branch.lastHour ?? shown);
      const at = reply && span && (!grouped || groups.data && !selected.reason)
        ? casualtyPairsAt(reply, shown, tag => !grouped || selected.tags === null || selected.tags.has(tag), between) : null;
      const exact = at ? casualtyChord(at, Number(options.mostCountries), shown, labelAt(shown),
        tag => shown === hour ? color(tag) : tokens.muted, tokens.muted) : null;
      if (exact) exactFrames.current.set(exact.data, exact);
      frame = { data: exact?.data ?? null, playing,
        segment: JSON.stringify([identity, span?.firstHour, at?.flows.map(flow => flow.id), seek]) };
      lastHour = shown; lastSeek = seek;
      return frame;
    };
  }, [runtime.clock, runtime.tracks, track, widget, manifest.data, histories.data, groups.data, grouped, fallback,
    options.mostCountries, labelAt, color, hour, tokens.muted, identity, between]);
  const stack = useMemo(() => pairHistory(chartReplies, selectedIds), [chartReplies, selected]); // eslint-disable-line react-hooks/exhaustive-deps
  const colors = useMemo(() => selectedIds.map((_id, index) => tokens.categories[index % tokens.categories.length]), [selectedIds, tokens.categories]);
  const colorById = new Map(selectedIds.map((id, index) => [id, colors[index]]));
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
    { key: 'value', header: 'Casualties', label: 'Casualties', numeric: true, width: 'max-content',
      value: flow => flow.value, render: pairAmount },
    { key: 'read', header: 'Read', label: 'Read', value: flow => flow.readHour, render: flow => formatDateTime(flow.readHour) },
  ], [detailLabel]);
  useEffect(() => { dispatch({ type: 'close' }); setDetail(null); setChartRange(null); }, [runtime.client, identity, widget.time, options.show, options.pairSelection]);
  useEffect(() => {
    onDiagnostics([...new Set([...(manifest.data?.warnings ?? []), ...[...histories.data.values()].flatMap(reply => reply.warnings), ...(groups.data?.warnings ?? [])])]);
    return () => onDiagnostics(NO_WARNINGS);
  }, [manifest.data, histories.data, groups.data, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see casualties between countries." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign." />;
  if (manifest.error) return <WidgetState kind="error" message={manifest.error} onRetry={manifest.retry} />;
  if (needsGroups && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (manifest.data?.available === false) return <WidgetState kind="unavailable" message="Not in this recording" />;
  if (selection.reason) return <WidgetState kind="empty" message={selection.reason} />;
  if (!manifest.data || needsGroups && !groups.data) return <WidgetState kind="loading" label="Loading casualty pairs…" />;
  if (pinned && options.pairSelection && !savedSelection) return <WidgetState kind="unavailable" message="This saved pair selection belongs to another campaign or branch. Select pairs again in the chord." />;
  const table = <Table label="Recorded casualty pairs" rows={rows} columns={columns} rowKey={flow => flow.id}
    sort={{ column: 'value', direction: 'descending' }} hoursPerSecond={speed} cellBars={false} rankMarkers={false}
    menu={flow => [{ label: 'Show this read', run: () => pickDay(flow.readHour) }, jumpItem(runtime, track.id, flow.readHour)]}
    tooltip={flow => fitTooltip([pairTooltip([flow], detailLabel, detailHour)], height)} />;
  const positive = rows.reduce((sum, row) => sum + (row.value > 0n ? row.value : 0n), 0n);
  const subtotal = rows.some(row => row.incomplete);
  const body = <div className="casualty-pair-details">
    {pinned && <p className="chart-line-key">{formatDateTime(detailHour)} {!effectiveDrill.follow
      && <button type="button" className="button small" onClick={follow}>Follow playback</button>}</p>}
    {detailOutside ? <WidgetState kind="unavailable" message={`The chosen moment ${formatDateTime(detailHour)} is outside this campaign branch.`} />
      : detailError ? <WidgetState kind="error" message={detailError} onRetry={histories.retry} />
      : !detailData ? <WidgetState kind="loading" label="Loading this moment…" />
        : !detailData.observed.some(span => span.firstHour <= detailHour && detailHour <= span.lastHour)
          ? <p className="drill-empty">Casualty pairs were not recorded at {formatDateTime(detailHour)}.</p>
          : !rows.length ? <p className="drill-empty">No selected native pair lifetimes were recorded at {formatDateTime(detailHour)}.</p>
            : <><DrillParts parts={rows.map(flow => ({ id: flow.id, label: `${detailLabel(flow.from).name} → ${detailLabel(flow.to).name}`,
              countryFlag: detailLabel(flow.from), raw: flow.value, value: Number(flow.value), text: pairAmount(flow), color: colorById.get(flow.id),
              shareText: flow.value > 0n && positive > 0n ? `${Number(flow.value * 1000n / positive) / 10}%` : '—' }))}
              format={value => value.toLocaleString('en-US')} virtual positiveBars showShares={!subtotal}
              totalRow={{ label: subtotal ? 'Recorded subtotal' : 'Signed total', value: rows.reduce((sum, row) => sum + row.value, 0n).toLocaleString('en-US') }} />
              <div className="casualty-pair-table">{table}</div></>}
    <div className="casualty-pair-history"><TimeChart label="Positive recorded casualties by directed pair over time"
      extent={[track.branch.firstHour, track.branch.lastHour]} branchStart={track.branch.firstHour}
      playhead={effectiveDrill.follow ? clockHour : null} playback="cursor" hoursPerSecond={speed}
      axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers}
      keyHours={stack.hours} active={active && open} revision={stack} pickOwner={pickOwner} onViewChange={setChartRange}
      onPick={pickDay} overlay={(ctx, frame, theme) => drawPlayhead(ctx, frame, theme, detailHour)}
      onContextMenu={(at, x, y) => menu.open(x, y, [{ label: 'Show this day', run: () => pickDay(at) }, jumpItem(runtime, track.id, Math.floor(at))])}
      tooltip={at => {
        const index = casualtyPairWindows(manifest.data, at, false)[0];
        const reply = index === undefined ? undefined : histories.data.get(index);
        const flows = reply ? casualtyPairsAt(reply, at).flows.filter(flow => selected.has(flow.id)) : [];
        return { title: formatDateTime(Math.floor(at)), rows: [{ label: flows.some(flow => flow.incomplete) ? 'Recorded subtotal' : 'Signed total',
          value: reply?.observed.some(span => span.firstHour <= at && at <= span.lastHour)
            ? flows.length ? flows.reduce((sum, flow) => sum + flow.value, 0n).toLocaleString('en-US') : 'No selected pair lifetimes'
            : reply ? 'Not recorded' : 'Loading…' }], note: 'Click to select this moment’s exact signed rows.' };
      }} /></div>
    {chartWindows.some(index => !histories.data.has(index) && !histories.errors.has(index)) && <p className="drill-empty">Loading history in the visible time range…</p>}
    {chartWindows.some(index => histories.errors.has(index)) && <button type="button" className="button small" onClick={histories.retry}>Retry history</button>}
    <p className="chart-line-key">Only positive amounts have geometry. Signed corrections and recorded zero remain exact in the table.</p>
  </div>;
  const main = outside ? <WidgetState kind="unavailable" message={`The chosen moment ${formatDateTime(hour)} is outside this campaign branch.`} />
    : error ? <WidgetState kind="error" message={error} onRetry={histories.retry} />
    : !data ? <WidgetState kind="loading" label="Loading casualty pairs…" />
      : filter.reason ? <WidgetState kind="empty" message={filter.reason} />
        : !data.observed.some(span => span.firstHour <= hour && hour <= span.lastHour)
          ? <WidgetState kind="unavailable" message="Casualty pairs were not recorded at this moment." />
          : !snapshot.flows.length ? <WidgetState kind="empty" message="No recorded country pairs at this moment." /> : <>
    <ChordChart label="Casualties between countries" caption="Ribbon colour = the country dealing casualties"
      frame={{ data: null }} frameAt={frameAt}
      hoursPerSecond={speed} active={active && (!open || options.drill === 'popout') && !pinned} onError={setChartError}
      tooltip={(hit, frame) => { const exact = exactFrames.current.get(frame);
        return exact ? fitTooltip([pairTooltip(chordPairs(hit, exact), exact.labels, exact.hour)], height) : null; }}
      onPick={(hit, frame) => { const exact = exactFrames.current.get(frame);
        if (exact) openPairs(chordPairs(hit, exact).map(flow => flow.id), exact.hour); }}
      onContextMenu={(hit, frame, x, y) => {
        const exact = exactFrames.current.get(frame);
        const flows = exact ? chordPairs(hit, exact) : [];
        if (flows.length) menu.open(x, y, [{ label: 'Recorded pairs', icon: 'grid', run: () => openPairs(flows.map(flow => flow.id), exact!.hour) },
          jumpItem(runtime, track.id, exact!.hour)]);
      }} />
    {chartError && <WidgetState kind="error" message={chartError} />}
    {!snapshot.flows.some(flow => flow.value > 0n) && <p className="chart-line-key">No positive attributable casualties are recorded at this moment.</p>}
    <button type="button" className="button small" onClick={() => openPairs(snapshot.flows.map(flow => flow.id), hour)}>All recorded pairs</button>
    {snapshot.flows.some(flow => flow.value < 0n) && <button type="button" className="button small"
      onClick={() => openPairs(snapshot.flows.filter(flow => flow.value < 0n).map(flow => flow.id), hour)}>Signed corrections</button>}
    <p className="chart-line-key">Across all recorded wars and military branches. Negative corrections are listed with their exact signs.
      {data.hasGap ? ' Missing hours may hide losses or entire wars; affected amounts are recorded subtotals.' : ''}</p>
  </>;
  return <div className="casualty-pairs" ref={anchor}>
    {pinned ? body : <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={open} title="Recorded casualty pairs"
      day={formatDateTime(detailHour)} following={effectiveDrill.follow} canFollow onFollow={follow}
      onPin={selectedIds.length ? () => runtime.addWidgetBelow(widget.id, pinPairs(widget, track, selectedIds, detailHour, effectiveDrill.follow)) : undefined}
      onClose={() => { dispatch({ type: 'close' }); setDetail(null); }}
      returnFocus=".chord-chart canvas" anchor={anchor} main={main}>{body}</DrillHost>}
    {menu.menu}
  </div>;
}
