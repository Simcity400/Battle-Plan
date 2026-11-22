import { countryFromKey } from '../../dashboard/countryKey.ts';
import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import type { EventBadge } from '../../charts/badges';
import { dayLabel, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import type { LegendItem } from '../../charts/Legend';
import { SECONDARY_DASH } from '../../charts/palette';
import { runAt } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer } from '../../charts/TimeChart';
import { CHART_LAYOUT } from '../../charts/tokens';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { countryLineColours, uncolouredWarning } from '../../dashboard/countryLineColours';
import { useCountryLineStyles } from '../../dashboard/useCountryLineStyles';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, membershipChanges, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { powerDecimals } from '../../dashboard/powerDecimals';
import { useIdentityHour } from '../../dashboard/atHead.ts';
import { followStore } from '../../dashboard/follow/store.ts';
import { shownThrough } from '../../dashboard/follow/store.ts';
import type { FollowEntry } from '../../dashboard/follow/store';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { FollowedText } from '../../dashboard/follow/FollowedText.tsx';
import { paintMeasureRow, rowAmount } from '../../dashboard/follow/painters.ts';
import { followRefusal, followsHead, usePlaybackHour, useFollow } from '../../dashboard/follow/useFollow.ts';
import { followAtHead } from '../../dashboard/tiles/head.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { OptionField } from '../../dashboard/options';
import { drawLineSummary, summaryExtent } from '../../charts/chartSummary';
import type { ChartSummary } from '../../charts/chartSummary';
import type { MarkCluster, MarkSummary } from '../../charts/markSummary';
import { changesRange, formatBalance, netSpend, rawOf, summaryEnd } from './measureData';
import { MEASURE_OPTIONS } from './options';
import { useMeasureSeries } from './measureSeries';
import BalanceChanges from './BalanceChanges';
import './measure.css';
import { motionOf } from '../../charts/easing.ts';
import { useChangedState } from '../../dashboard/changedState.ts';

const NO_WARNINGS: string[] = [];
const NO_FLAGS: readonly string[] = [];
const NO_PAINT = () => {};
const CEILING_ROWS_MAX = 4;

export interface MeasureConfig {
  measure: 'politicalPower' | 'commandPower';
  title: string;
  noun: string;
  unit: string;
  options?: readonly OptionField[];
  ceiling?: string;
}

export default function MeasureWidget({ widget, runtime, active, onDiagnostics, config }: WidgetProps & { config: MeasureConfig }) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(config.options ?? MEASURE_OPTIONS, widget.options);
  const showCeiling = config.ceiling != null && options.ceiling !== false;
  const digits = powerDecimals(options);
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const [visibleFlags, setVisibleFlags] = useChangedState<readonly string[]>(NO_FLAGS);
  const onVisibleItems = useCallback((items: readonly LegendItem[]) => {
    const tags = [...new Set(items.map(item => item.id))].sort();
    setVisibleFlags(before => before.length === tags.length && before.every((tag, index) => tag === tags[index]) ? before : tags);
  }, []);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const fallback = defaultCountryKey(track);
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, lastHour]);
  const following = widget.time.mode === 'follow';
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track.source, runtime.clock, track.axisOffset) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const headQuery = useMemo(() => following && active && track && resolved.tags.length ? { product: 'series-state', params: { branchId: track.branch.id,
    family: { kind: 'country-measure', measure: config.measure }, reads: resolved.tags.map(tag => ({ country: countryFromKey(tag), hour: 0 })) } } : null,
  [following, active, track?.branch.id, config.measure, resolved.tags]);
  const shown = useShownHour();
  const followed = useFollow(store, headQuery, root, useCallback((entry: FollowEntry) => shown(shownThrough(entry)), [shown]));
  const followedHead = followsHead(followed);
  const ceilingQuery = useMemo(() => following && active && showCeiling && track && resolved.tags.length ? { product: 'series-state', params: { branchId: track.branch.id,
    family: { kind: 'country-stability', measure: 'maxCommandPower' }, reads: resolved.tags.map(tag => ({ country: countryFromKey(tag), hour: 0 })) } } : null,
  [following, active, showCeiling, track?.branch.id, resolved.tags]);
  const ceilingFollowed = useFollow(store, ceilingQuery, root, NO_PAINT);
  const ceilingHead = followsHead(ceilingFollowed);
  const headFollowed = followedHead && (!showCeiling || ceilingHead);
  const refused = followRefusal(followed, ceilingFollowed);
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = playhead ?? extent[1];
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const membership = resolved.membership ?? undefined;
  const balances = useMeasureSeries(runtime.client, track?.source, track?.branch.id, 'country-measure', config.measure, resolved.tags,
    following ? null : head, hoverHour, active, options.spendTicks === true, membership, lastHour, following);
  const ceiling = useMeasureSeries(runtime.client, showCeiling ? track?.source : undefined, track?.branch.id, 'country-stability', 'maxCommandPower',
    resolved.tags, following ? null : head, hoverHour, active && showCeiling, false, membership, lastHour, following);
  const [hoverCluster, setHoverCluster] = useChangedState<{ owner: unknown; cluster: MarkCluster | null } | null>(null);
  const [changeList, setChangeList] = useState<{ owner: unknown; id: number; tag: string; first: string; end?: string } | null>(null);
  const [changeLists, setChangeLists] = useState(0);
  const onMarkHover = useCallback((cluster: MarkCluster | null) => setHoverCluster(previous => previous?.owner === balances.owner && previous.cluster === cluster
    ? previous : { owner: balances.owner, cluster }), [balances.owner]);
  const identityHour = useIdentityHour(runtime.client, track, resolved.tags, runtime.clock, active && following && headFollowed,
    useCallback(hour => track ? resolveWindow(widget, track, runtime.tracks, hour)?.lastHour ?? null : null, [widget, track, runtime.tracks]));
  const styleAt = following && headFollowed && identityHour !== null ? identityHour
    : playhead ?? (widget.time.mode === 'range' && window ? window.lastHour : lastHour);
  const styles = useCountryLineStyles(runtime.client, track, resolved.tags, styleAt, active);
  const known = styles.known;
  const lineByTag = useMemo(() => new Map(balances.lines.map(line => [line.tag, line])), [balances.lines]);
  const ceilingByTag = useMemo(() => new Map(ceiling.lines.map(line => [line.tag, line])), [ceiling.lines]);
  const lookSegments = useCallback((tag: string, hour: number) => {
    const run = runAt(lineByTag.get(tag)?.runs ?? [], hour);
    return run?.segmentFirstHour ?? null;
  }, [lineByTag]);
  const flagTags = active ? visibleFlags : NO_FLAGS;
  const legendLooks = useCountryLooks(runtime.client, track, flagTags, styleAt, active, lookSegments, undefined, true, flagTags);

  useEffect(() => {
    const uncoloured = uncolouredWarning(resolved.tags, known, !!styles.data);
    const warnings = [...(styles.data?.warnings ?? []),
      ...(styles.error ? [`Map colours could not be loaded: ${styles.error.message}`] : []), ...(groups.data?.warnings ?? []),
      ...(widget.countries.kind === 'group' && groups.error ? [`Country membership labels could not be loaded: ${groups.error.message}`] : []),
      ...(showCeiling && ceiling.error ? [`The ${config.ceiling} could not be loaded: ${ceiling.error.message}`] : []),
      ...(uncoloured ? [uncoloured] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [resolved.tags, styles.data, styles.error, known, groups.data, groups.error, widget.countries.kind, onDiagnostics, showCeiling, ceiling.error, config.ceiling]);

  const decimals = balances.decimals;
  const readHead = !following && resolved.tags.length > 0 && resolved.tags.every(tag => balances.hourLoaded(tag, Math.floor(head)));
  useEffect(() => { if (!following) shown(readHead ? head : null); }, [following, readHead, head, shown]);
  const series = useMemo(() => {
    const colours = countryLineColours(resolved.tags, known, tokens.surface, tokens.countryColours === 'fixed');
    return resolved.tags.map(tag => {
      const line = lineByTag.get(tag);
      const spans = membership?.get(tag);
      const valueAt = (hour: number) => isMember(spans, hour) ? runAt(line?.runs ?? [], hour)?.valueRaw ?? null : null;
      const ceilingAt = (hour: number) => isMember(spans, hour) ? runAt(ceilingByTag.get(tag)?.runs ?? [], hour)?.valueRaw ?? null : null;
      return { tag, hex: colours.get(tag)!, membership: spans, valueAt, ceilingAt, drops: line?.drops ?? [], missing: line?.missing };
    });
  }, [resolved.tags, known, tokens, lineByTag, ceilingByTag, membership]);

  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const wantedLabels = useMemo(() => series.flatMap(item => [
    { tag: item.tag, hour: Math.floor(styleAt) },
    ...(hoverHour === null ? [] : [{ tag: item.tag, hour: hoverHour }]),
    ...(item.membership ? membershipChanges(item.membership, extent[0], extent[1], resolved.unobserved).map(change => ({ tag: item.tag, hour: change.hour })) : []),
  ]), [series, styleAt, hoverHour, extent[0], extent[1], resolved.unobserved]);
  const nameHours = useMemo(() => Math.max(MAX_NAME_HOURS, new Set(wantedLabels.map(item => item.hour)).size), [wantedLabels]);
  const names = useCountryNamesAt(runtime.client, track, wantedLabels, active, nameHours);
  const hoverLabels = useCountryLabelsAt(runtime.client, track, hoverHour === null ? [] : series.map(item => ({ tag: item.tag, hour: hoverHour })), active);
  const labelAt = useCallback((hour: number) => (tag: string) => {
    const name = names.get(nameKey(hour, tag));
    return { ...hoverLabels.labelAt(hour)(tag), ...(name ? nameReadMetadata(name) : {}) };
  }, [names, hoverLabels.labelAt]);

  const badges = useMemo<EventBadge[]>(() => series.flatMap(item => item.membership
    ? membershipChanges(item.membership, extent[0], extent[1], resolved.unobserved).map(change => ({
      hour: change.hour, glyph: change.change === 'joined' ? '+' : '−', label: `${labelAt(change.hour)(item.tag).name} ${change.change}`, color: item.hex })) : []),
  [series, extent[0], extent[1], resolved.unobserved, labelAt]);
  const menu = useContextMenu('Chart actions');
  const followedEntry = followedHead ? followed.entry : null;
  const ceilingEntry = ceilingHead ? ceilingFollowed.entry : null;
  useEffect(() => {
    const heads = [[followedEntry, balances.head, decimals], [ceilingEntry, ceiling.head, ceiling.decimals]] as const;
    const stops = heads.flatMap(([entry, head, places]) => entry && places !== null
      ? resolved.tags.map((tag, row) => followAtHead(head(tag), () => rowAmount(entry.answer, row, places))) : []);
    return () => stops.forEach(stop => stop());
  }, [followedEntry, ceilingEntry, resolved.tags, balances.head, ceiling.head, decimals, ceiling.decimals]);
  const shownAt = Math.floor(styleAt);
  const legendItems = useMemo(() => series.map((item, row) => {
    const at = Math.floor(head);
    const raw = item.valueAt(at);
    return { id: item.tag, label: labelAt(shownAt)(item.tag).name, color: item.hex,
      value: followedEntry && store
        ? decimals === null ? 'Loading…' : <FollowedText entry={followedEntry} write={(entry, cell) => paintMeasureRow(entry.answer, cell, row, decimals, digits)} />
        : raw == null || decimals === null ? balances.hourLoaded(item.tag, at) ? 'Not recorded' : 'Loading…' : formatBalance(raw, decimals, digits),
      flag: <CountryFlagUrl className="country-flag" url={legendLooks.flag(item.tag, shownAt)}
        pending={legendLooks.flagPending(item.tag, shownAt)} detail={legendLooks.flagDetail?.(item.tag, shownAt)} reason={legendLooks.flagReason(item.tag, shownAt)} /> };
  }), [series, head, shownAt, labelAt, legendLooks, digits, decimals, balances, followedEntry, store]);
  const markSummaries = useMemo(() => {
    if (!options.spendTicks) return undefined;
    const drawn = new WeakMap<MarkSummary, MarkSummary>();
    let held: { marks: readonly (MarkSummary | null)[]; list: readonly MarkSummary[] } | null = null;
    return () => {
      const marks = series.map(item => balances.marksOf(item.tag));
      if (held && held.marks.every((mark, at) => mark === marks[at])) return held.list;
      const list = series.flatMap((item, at) => {
        const mark = marks[at];
        if (!mark) return [];
        let made = drawn.get(mark);
        if (!made) drawn.set(mark, made = { ...mark, placement: 'ticks' as const, color: series.length === 1 ? tokens.muted : item.hex,
          clusters: mark.clusters.map(cluster => ({ ...cluster, tag: item.tag })) });
        return [made];
      });
      held = { marks, list };
      return list;
    };
  }, [balances.marksOf, options.spendTicks, series, tokens.muted]);
  const chartData = useMemo<ChartData>(() => ({ revision: () => [balances.revision(), ceiling.revision()],
    subscribe: listener => { const stops = [balances.subscribe(listener), ceiling.subscribe(listener)]; return () => stops.forEach(stop => stop()); } }),
  [balances.revision, balances.subscribe, ceiling.revision, ceiling.subscribe]);

  const layers = useMemo<ChartLayer[]>(() => {
    const single = series.length === 1;
    const line = (summary: () => ChartSummary | undefined, hex: string, dash?: readonly number[], area = false): ChartLayer => ({
      draw: (ctx, frame, chartTokens, state) => {
        const shown = summary();
        if (shown) drawLineSummary(ctx, frame, chartTokens, shown, hex, false, state.playhead, dash ? { dash } : { area });
      },
      extent: (x: readonly [number, number]) => {
        const shown = summary();
        return shown ? summaryExtent(shown, x, until) : null;
      },
    });
    const list: ChartLayer[] = series.map(item => line(() => balances.lineOf(item.tag), item.hex, undefined, single && options.areaFill === true));
    if (showCeiling) for (const item of series) list.push(line(() => ceiling.lineOf(item.tag), item.hex, SECONDARY_DASH));
    return list;
  }, [series, options.areaFill, until, balances.lineOf, ceiling.lineOf, showCeiling]);

  if (!track) return <WidgetState kind="empty" icon="sparkle" message={`Load a campaign to see ${config.noun}.`} />;
  if (refused) return <WidgetState kind="unavailable" message={refused} />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (widget.countries.kind === 'group' && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  if (balances.error && !balances.summaries) return <WidgetState kind="error" failure={balances.error} onRetry={balances.retry} />;
  if (widget.countries.kind === 'group' && !groups.data) return <WidgetState kind="loading" label={`Loading ${config.noun}…`} />;

  return <div className="country-measure" ref={setRoot}><div className="country-measure-view">
    {balances.error && <WidgetState kind="error" failure={balances.error} onRetry={balances.retry} />}
    <Legend label="Countries" items={legendItems} windowed onVisibleItems={onVisibleItems} />
    { }
    {showCeiling && <p className="chart-line-key"><i className="is-dashed" aria-hidden="true" />Dashed line: {config.ceiling}</p>}
    <div className="country-measure-chart">
      <TimeChart label={`${config.title} of ${series.map(item => labelAt(shownAt)(item.tag).name).join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour} liveHead={liveHead}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        padding={{ right: options.endLabels ? CHART_LAYOUT.measureEndLabels : CHART_LAYOUT.right }} layers={layers} active={active} notRecorded={balances.notRecorded}
        revision={`${head}:${track.branch.id}:${series.length}:${viewport}:${digits}:${balances.states ? 'states' : 'loading'}`} data={chartData}
        pickOwner={balances.owner}
        onSummaryView={view => { balances.onSummaryView(view); if (showCeiling) ceiling.onSummaryView(view); }}
        onNavigateHour={balances.onNavigateHour}
        keyboardHour={balances.keyboardHour} keyboardRevision={balances.keyboardRevision}
        onHover={setHoverHour}
        markSummaries={markSummaries}
        onMarkHover={onMarkHover}
        markTooltip={cluster => {
          const label = cluster.tag ? labelAt(cluster.firstHour)(cluster.tag) : null;
          const drop = cluster.tag && series.find(item => item.tag === cluster.tag)?.drops.find(drop => drop.hour === cluster.firstHour);
          return { title: cluster.firstHour === cluster.lastHour ? dayLabel(cluster.firstHour) : `${dayLabel(cluster.firstHour)}–${dayLabel(cluster.lastHour)}`,
            countryFlag: label ?? undefined, subtitle: label?.name,
            rows: [{ label: 'Recorded balance changes', value: cluster.count },
              ...(drop ? [{ label: 'Balance fell', value: netSpend(drop, decimals!, digits) }] : [])],
            note: 'Open Balance changes from the chart menu to reach each event.' };
        }}
        formatValue={value => Math.abs(value) >= 1000 ? `${Math.round(value / 100) / 10}k` : String(Math.round(value))}
        badges={badges.length ? badges : undefined}
        tooltip={(hour, _frame, hovered) => {
          const rows: TooltipRow[] = series.flatMap(item => {
            const raw = item.valueAt(hour);
            const label = labelAt(hour)(item.tag);
            const missing = lineByTag.get(item.tag)?.missing.get(Math.floor(hour));
            const balance: TooltipRow = { label: label.name, countryFlag: label, swatch: item.hex,
              value: raw == null ? !balances.hourLoaded(item.tag, Math.floor(hour)) ? 'Loading balance…' : 'Not recorded' : `${formatBalance(raw, decimals!, digits)} ${config.unit}`,
              detail: raw == null ? missing : undefined, tone: raw == null ? 'muted' : undefined };
            if (!showCeiling || series.length > CEILING_ROWS_MAX) return [balance];
            const maximum = item.ceilingAt(hour);
            return [balance, { label: series.length > 1 ? `${label.name}: maximum` : 'Maximum', value: maximum == null ? 'Not recorded' : formatBalance(maximum, ceiling.decimals!, digits), tone: 'muted' }];
          });
          for (const item of series) {
            const drop = item.drops.find(entry => entry.hour === Math.floor(hour));
            if (drop) rows.push({ label: series.length > 1 ? `${labelAt(hour)(item.tag).name}: balance fell` : 'Balance fell', value: netSpend(drop, decimals!, digits), divider: true });
          }
          rows.push(...badgeRows(hovered));
          return fitTooltip([{ title: dayLabel(hour), rows }], viewport);
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          const at = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          const ends = decimals === null ? [] : series.flatMap(item => {
            const value = summaryEnd(balances.lineOf(item.tag), at);
            if (value == null) return [];
            const raw = rawOf(value, decimals);
            const text = series.length === 1 ? { text: `${formatBalance(raw, decimals, digits)} ${config.unit}` }
              : { text: labelAt(shownAt)(item.tag).name, suffix: formatBalance(raw, decimals, digits) };
            return [{ id: item.tag, y: frame.toY(value), ...text, color: item.hex }];
          });
          placeEndLabels(ctx, frame, chartTokens, ends);
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour), ...(series.length ? [{ label: 'Balance changes', run: () => {
          const cluster = hoverCluster?.owner === balances.owner ? hoverCluster.cluster : null;
          const tag = cluster?.tag ?? series[0].tag;
          const drawnUntil = shownUntil(liveHead && playhead != null ? liveHead.read() : playhead, playback, options.afterPlayhead === true);
          const { first, end } = changesRange(balances.marksOf(tag), cluster, drawnUntil);
          setChangeLists(count => count + 1);
          setChangeList({ owner: balances.owner, id: changeLists + 1, tag, first, end });
        } }] : [])])} />
    </div>
    {changeList?.owner === balances.owner && <BalanceChanges key={changeList.id} client={runtime.client} source={track.source} branchId={track.branch.id}
      measure={config.measure} tags={resolved.tags} initialTag={changeList.tag} firstOrdinal={changeList.first} endOrdinal={changeList.end}
      digits={digits} unit={config.unit} visibleThrough={until ?? extent[1]} label={tag => labelAt(shownAt)(tag).name}
      onClose={() => setChangeList(null)} onPick={(_tag, hour) => jumpItem(runtime, track.id, hour).run()} />}
    {menu.menu}
  </div></div>;
}
