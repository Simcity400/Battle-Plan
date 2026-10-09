import { useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { drawBands } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import { jumpItem } from '../../charts/jump';
import { stackedSegments, stackExtent } from '../../charts/stack';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/Sunburst';
import { sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { flagReadSpec } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryFactsRead, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import type { CasualtyBreakdownReply } from '../../generated/dashboard';
import { breakdownWindows, exactLosses, lossItems, lossesAt, lossStack, lossText, validateBreakdown } from './casualtiesBreakdownData';
import type { LossRow } from './casualtiesBreakdownData';
import { casualtyCountryCell } from './casualtyFlags';
import { definition, DETAIL } from './definition';
import './casualtiesBreakdown.css';

const ROOT = { id: 'positive-losses', label: 'Positive known losses' };
const NO_WARNINGS: string[] = [];
const TEXT: SunburstText = { format: (value, exact) => exactLosses(exact ?? BigInt(Math.round(value))), unit: 'casualties', valueLabel: 'Positive amounts' };

export default function CasualtiesBreakdownWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const speed = usePlaybackSpeed(runtime.clock), tokens = useChartTokens(), height = useViewportHeight();
  const options = resolveOptions(definition.options, widget.options);
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Casualty breakdown actions');
  const [drill, setDrill] = useState(CLOSED);
  const playback = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const [visible, setVisible] = useState<readonly LossRow[]>([]);
  const [chartRange, setChartRange] = useState<readonly [number, number] | null>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
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
  const pickDay = (at: number) => setDrill(state => reduceDrill(state.breakdown ? state : effectiveDrill, { type: 'pick', hour: at }));
  const follow = () => setDrill(state => reduceDrill(state.breakdown ? state : effectiveDrill, { type: 'follow' }));
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'casualty-breakdown', params: { branchId: track.branch.id, tag, window: null } } : null;
  const manifest = useProduct<CasualtyBreakdownReply>(runtime.client, request, active, validateBreakdown);
  const currentWindows = breakdownWindows(manifest.data, hour, widget.time.mode === 'follow');
  const detailWindows = breakdownWindows(manifest.data, detailHour, effectiveDrill.follow);
  const finalIndex = manifest.data ? manifest.data.windows.length - 1 : -1;
  // Only the time-chart viewport requests history blocks. At the default full
  // branch extent they all load; zooming releases the blocks no longer shown.
  const chartWindows = open ? manifest.data?.windows.flatMap((span, index) => !chartRange
    || span.firstHour <= chartRange[1] && chartRange[0] <= span.lastHour ? [index] : []) ?? [] : [];
  const wanted = [...new Set([...currentWindows, ...detailWindows, ...chartWindows, ...(finalIndex >= 0 ? [finalIndex] : [])])].sort((a, b) => a - b);
  const identity = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]) : null;
  const histories = useProductWindows<CasualtyBreakdownReply>(runtime.client, identity, index => ({ ...request!,
    params: { branchId: track!.branch.id, tag: tag!, window: index } }), wanted, active && manifest.data?.available === true, validateBreakdown);
  const current = currentWindows[0] === undefined ? undefined : histories.data.get(currentWindows[0]);
  const detail = detailWindows[0] === undefined ? undefined : histories.data.get(detailWindows[0]);
  const final = histories.data.get(finalIndex);
  const snapshot = useMemo(() => current ? lossesAt(current, hour) : null, [current, hour]);
  const details = useMemo(() => detail ? lossesAt(detail, detailHour) : null, [detail, detailHour]);
  const end = useMemo(() => final && track ? lossesAt(final, track.branch.lastHour) : null, [final, track]);
  const nameFacts = useMemo(() => [...(snapshot?.rows ?? []), ...(details?.rows ?? []), ...(end?.rows ?? [])]
    .map(row => ({ tag: row.enemyTag, hour: row.enemyHour })), [snapshot, details, end]);
  const names = useCountryNamesAt(runtime.client, track, nameFacts, active);
  const enemyName = (row: LossRow) => names.get(nameKey(row.enemyHour, row.enemyTag)) ?? row.enemyTag;
  // Artwork follows the table viewport, including a removed enemy's last
  // observed holder. A replacement under its tag cannot supply its old flag.
  const flagFacts = useMemo(() => visible.map(row => ({ tag: row.enemyTag, hour: row.enemyHour })), [visible]);
  const flags = useCountryFactsRead(runtime.client, track, flagFacts, active && open, flagReadSpec('small'));
  const items = useMemo(() => lossItems(snapshot, end, (tag, hour) => names.get(nameKey(hour, tag)) ?? tag), [snapshot, end, names]);
  const levels = [options.warLevel !== false, options.enemyLevel !== false];
  const chartReplies = useMemo(() => chartWindows.flatMap(index => histories.data.has(index) ? [histories.data.get(index)!] : []),
    [chartWindows.join(','), histories.data]); // eslint-disable-line react-hooks/exhaustive-deps
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
  }], [stack, colors.join(',')]); // eslint-disable-line react-hooks/exhaustive-deps
  const columns = useMemo<TableColumn<LossRow>[]>(() => [
    { key: 'war', header: 'War', label: 'War', value: row => row.war },
    { key: 'enemy', header: 'Enemy', label: 'Enemy', value: row => names.get(nameKey(row.enemyHour, row.enemyTag)) ?? row.enemyTag,
      render: row => <CountryCell {...casualtyCountryCell(row, names, flags)} /> },
    { key: 'losses', header: 'Known taken', label: 'Known taken', numeric: true, width: 'max-content', value: row => row.value, render: lossText },
    { key: 'read', header: 'Read', label: 'Read', width: 'max-content', value: row => row.readHour, render: row => formatDateTime(row.readHour) },
  ], [names, flags]);
  useEffect(() => { setDrill(CLOSED); setVisible([]); setChartRange(null); }, [runtime.client, identity, widget.time, options.show]);
  useEffect(() => {
    onDiagnostics([...new Set([...(manifest.data?.warnings ?? []), ...[...histories.data.values()].flatMap(reply => reply.warnings)])]);
    return () => onDiagnostics(NO_WARNINGS);
  }, [manifest.data, histories.data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its casualties." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign." />;
  if (manifest.error) return <WidgetState kind="error" message={manifest.error} onRetry={manifest.retry} />;
  if (!manifest.data) return <WidgetState kind="loading" label="Loading casualty breakdown…" />;
  if (!manifest.data.available) return <WidgetState kind="unavailable" message="The native war grouping needed for this breakdown was not recorded." />;
  const currentError = currentWindows[0] === undefined ? undefined : histories.errors.get(currentWindows[0]);
  const error = detailWindows[0] === undefined ? undefined : histories.errors.get(detailWindows[0]);
  const summary = (value: typeof snapshot) => value
    ? <p className="casualty-breakdown-total">{value.incomplete ? 'Recorded subtotal' : 'Known taken'}: <b>{exactLosses(value.total)}</b></p> : null;
  const detailBody = <div className="casualty-breakdown-details">
    {pinned && <p className="casualty-breakdown-total">{formatDateTime(detailHour)} {!effectiveDrill.follow
      && <button type="button" className="button small" onClick={follow}>Follow playback</button>}</p>}
    {detailOutside ? <WidgetState kind="unavailable" message={`The chosen moment ${formatDateTime(detailHour)} is outside this campaign branch.`} />
      : error ? <WidgetState kind="error" message={error} onRetry={histories.retry} />
      : !detail ? <WidgetState kind="loading" label="Loading this moment…" />
        : !details ? <p className="drill-empty">{detail.observed.some(span => span.firstHour <= detailHour && detailHour <= span.lastHour)
          ? 'No casualty relations were recorded for this country' : 'Casualty counters were not recorded'} at {formatDateTime(detailHour)}.</p>
          : <>{summary(details)}<div className="casualty-breakdown-table"><Table label="Recorded casualty lifetimes" rows={details.rows}
            columns={columns} rowKey={row => row.id} sort={{ column: 'losses', direction: 'descending' }}
            cellBars={false} rankMarkers={false} hoursPerSecond={speed} onVisibleRows={setVisible}
            menu={row => [jumpItem(runtime, track.id, row.readHour)]}
            tooltip={row => fitTooltip([{ title: `${row.war} › ${enemyName(row)}`, rows: [
              { label: 'Known taken', value: lossText(row), strong: true },
              { label: 'Read', value: formatDateTime(row.readHour) },
              { label: 'Native grouping supplied', value: formatDateTime(row.groupingHour) },
              ...(row.enemyHour < detailHour ? [{ label: 'Enemy facts read', value: formatDateTime(row.enemyHour) }] : []),
            ], note: [row.reason, row.nameReason, row.incomplete ? 'Missing hours may hide additional losses or whole wars.' : null].filter(Boolean).join(' ') }], height)} /></div></>}
    <div className="casualty-breakdown-history"><TimeChart label="Positive recorded casualties by war over time"
      extent={[track.branch.firstHour, track.branch.lastHour]} branchStart={track.branch.firstHour}
      playhead={effectiveDrill.follow ? clockHour : null} playback="cursor" hoursPerSecond={speed}
      axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers}
      active={active && open} revision={stack} onViewChange={setChartRange}
      onPick={pickDay}
      onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, Math.floor(at))])}
      tooltip={at => {
        const index = breakdownWindows(manifest.data, at, false)[0];
        const reply = index === undefined ? undefined : histories.data.get(index);
        const value = reply ? lossesAt(reply, at) : null;
        return { title: formatDateTime(Math.floor(at)), rows: [{ label: value?.incomplete ? 'Recorded subtotal' : 'Known taken',
          value: value ? exactLosses(value.total) : reply ? 'Not recorded' : 'Loading…', strong: true }],
          note: 'Bands show positive amounts; click to select this moment’s exact signed rows.' };
      }} /></div>
    {chartWindows.some(index => !histories.data.has(index) && !histories.errors.has(index)) && <p className="drill-empty">Loading history in the visible time range…</p>}
    {chartWindows.some(index => histories.errors.has(index)) && <button type="button" className="button small" onClick={histories.retry}>Retry history</button>}
    <p className="chart-line-key">Only positive amounts have geometry. Exact signed corrections and zero values remain in the table.</p>
  </div>;
  const main = outside ? <WidgetState kind="unavailable" message={`The chosen moment ${formatDateTime(hour)} is outside this campaign branch.`} />
    : currentError ? <WidgetState kind="error" message={currentError} onRetry={histories.retry} />
      : !current ? <WidgetState kind="loading" label="Loading casualty history…" />
        : <div className="casualty-breakdown-main" onContextMenu={event => { event.preventDefault(); menu.open(event.clientX, event.clientY, [jumpItem(runtime, track.id, hour)]); }}
    onKeyDown={event => { if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) {
      event.preventDefault(); const box = anchor.current?.getBoundingClientRect(); if (box) menu.open(box.left, box.top, [jumpItem(runtime, track.id, hour)]);
    } }}>
    {summary(snapshot)}
    {!snapshot ? <WidgetState kind={current.observed.some(span => span.firstHour <= hour && hour <= span.lastHour) ? 'empty' : 'unavailable'} icon="sunburst"
      message={current.observed.some(span => span.firstHour <= hour && hour <= span.lastHour)
        ? 'No casualty relations were recorded for this country at this moment.' : 'Casualty counters were not recorded at this moment.'} />
      : <Sunburst key={JSON.stringify([identity, snapshot.identity, manifest.data.observed.find(span => span.firstHour <= hour && hour <= span.lastHour)?.firstHour,
        widget.time.mode === 'follow' ? runtime.clock.getSnapshot().seekSequence : 0])}
        label="Positive known casualties by native war and enemy" root={ROOT} items={items} levels={levels} text={TEXT}
        settings={sunburstSettings(options)} hoursPerSecond={speed} active={active && (!open || options.drill === 'popout')}
        footnote="Shares show positive amounts only. Known losses from land, naval and air are combined; losses with no recorded attacker are excluded." />}
    <button type="button" className="button small" onClick={() => setDrill(state => reduceDrill(state,
      { type: 'open', breakdown: DETAIL.id, hour, playing: widget.time.mode === 'follow' && runtime.clock.getSnapshot().playing,
        playhead: widget.time.mode === 'follow' ? hour : null }))}>All recorded losses{snapshot?.rows.some(row => row.value < 0n) ? ' and signed corrections' : ''}</button>
    {manifest.data.hasGap && <p className="chart-line-key">Missing hours may hide losses or whole wars. Affected values are recorded subtotals.</p>}
  </div>;
  return <div ref={anchor} className="casualty-breakdown">
    {pinned ? detailBody : <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={open} title="Recorded losses"
      day={formatDateTime(detailHour)} following={effectiveDrill.follow} canFollow
      onClose={() => setDrill(CLOSED)} onFollow={follow}
      onPin={runtime.addWidgetBelow ? () => {
        const pin = pinnedBreakdown(widget, DETAIL, track.id);
        const offset = Math.floor(detailHour) - track.branch.firstHour;
        runtime.addWidgetBelow?.(widget.id, { ...pin, countries: { kind: 'tags', tags: [tag] },
          time: effectiveDrill.follow ? { mode: 'follow' } : { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } } });
      } : undefined}
      anchor={anchor} main={main}>{detailBody}</DrillHost>}
    {menu.menu}
  </div>;
}
