import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import LiveText, { liveTexts, separated } from '../../charts/LiveText';
import { markIndex } from '../../charts/markSummary';
import type { MarkCluster, MarkSummary } from '../../charts/markSummary';
import { categoricalPalette } from '../../charts/palette';
import TimeChart from '../../charts/TimeChart';
import type { ChartData } from '../../charts/TimeChart';
import { tiledStackLayer } from '../../charts/tiledStack';
import type { TooltipRow } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { loadTransientProduct, useProduct } from '../../dashboard/products';
import { useAtHead } from '../../dashboard/atHead';
import { useKeyedSeries } from '../../dashboard/keyedSeries';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { settledHour, trackHourOf } from '../../dashboard/settledHour';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { readTokens, subscribeTokens } from '../../charts/tokens';
import { OPTIONS } from './definition';
import { POPULARITY_FAMILY, POPULARITY_PART_BITS, bandKeys, bands, formatPopularity, formatShare, leaderAt, leaderShown,
  partyBadges, partyChangeHours, partyNameAt, rawAt, rulingAt, rulingFirst, stepHour, validatePartyHistoryReply, bandSummaries } from './partyData';
import type { Band } from './partyData';
import { governmentBadges, governmentChanges, governmentRows, visibleGovernmentChanges } from './governmentData';
import PartyEventList, { GovernmentChangeList } from './PartyEventList';
import CountryPeople from '../country-people/CountryPeople';
import './rulingParty.css';
import { motionOf } from '../../charts/easing.ts';
import type { PartyHistoryReply } from '../../generated/dashboard';
import { useChangedState } from '../../dashboard/changedState.ts';

const NO_WARNINGS: string[] = [];
export const EVENT_PAGE = 128;

export default function RulingPartyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const root = useRef<HTMLDivElement>(null);
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'party-history',
    params: { branchId: track.branch.id, country: countryFromKey(tag) } } : null;
  const { data, error, retry } = useProduct<PartyHistoryReply>(runtime.client, request, active, validatePartyHistoryReply);
  const slots = useSlotTags(runtime.client, track, active && !!tag && isSlotKey(tag));
  const live = widget.time.mode === 'follow';
  const following = live || widget.time.mode === 'pin';
  const window = track && !live ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const changes = useMemo(() => data ? partyChangeHours(data) : [], [data]);
  useShownEachFrame(runtime.clock, () => data ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const settled = useAtHead(runtime.clock, active && live && !!data, clockHour => {
    const hour = trackHourOf(widget, track, runtime.tracks, clockHour);
    return hour === null ? null : settledHour(changes, hour);
  });
  const shownHour = live ? settled : trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour());
  const playhead = following ? shownHour : null;
  const head = shownHour ?? extent[1];
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => live ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [live, runtime.clock, axisOffset]);
  const [hover, setHover] = useState<number | null>(null);
  const [surface, setSurface] = useState(() => readTokens().surface);
  useEffect(() => subscribeTokens(tokens => setSurface(tokens.surface)), []);
  const ordered = useMemo(() => data && tag ? bands(data, tag, categoricalPalette(surface)) : [], [data, tag, surface]);
  const keys = useMemo(() => bandKeys(ordered), [ordered]);
  const rulingShown = data ? rulingAt(data, head) : null;
  const list = useMemo(() => rulingFirst(ordered, rulingShown), [ordered, rulingShown]);
  const spec = useMemo(() => track && keys.length ? { source: track.source, branchId: track.branch.id, family: POPULARITY_FAMILY, keys,
    partBits: POPULARITY_PART_BITS, lastHour: track.branch.lastHour } : null, [track?.source, track?.branch.id, track?.branch.lastHour, keys]);
  const popularity = useKeyedSeries(runtime.client, runtime.clock, spec, live ? null : head, hover, live, active, root);
  const names = useMemo(() => {
    const names = new Map((data?.ideologies ?? []).map(ideology => [ideology.token, contentTitle(ideology.name, ideology.token, 'Ideology name unavailable')]));
    for (const band of ordered) names.set(band.token, band.name);
    return names;
  }, [data, ordered]);
  const name = useCallback((token: string) => names.get(token) ?? 'Ideology name unavailable', [names]);
  const detailsThrough = shownUntil(playhead, (options.playback as PlaybackMode) ?? 'draw-in', options.afterPlayhead === true) ?? extent[1];
  const governments = governmentChanges();
  const events = useMemo(() => {
    if (!data) return [];
    const all = partyBadges(data, name, options.leaders === true, governments);
    return options.changes ? [...all, ...governmentBadges(governments, name, extent[1])].sort((a, b) => a.hour - b.hour)
      : all.filter(badge => badge.glyph === '♔');
  }, [data, name, options.changes, options.leaders, governments, extent[1]]);
  const index = useMemo(() => markIndex(events), [events]);
  const markHours = useMemo(() => [...new Set(events.map(event => event.hour))].sort((a, b) => a - b), [events]);
  const view = useRef<{ from: number; end: number; width: number } | null>(null);
  const marks = useRef<{ index: typeof index; from: number; end: number; width: number; summary: MarkSummary } | null>(null);
  const markSummaries = useCallback((): readonly MarkSummary[] => {
    const at = view.current;
    if (!at || !events.length) return [];
    const held = marks.current;
    if (held && held.index === index && held.from === at.from && held.end === at.end && held.width === at.width) return [held.summary];
    const summary: MarkSummary = { ...index.summary(at.from, at.end, at.width), tiles: true };
    marks.current = { index, ...at, summary };
    return [summary];
  }, [index, events.length]);
  const onSummaryView = useCallback((next: { from: number; to: number; end: number; width: number }) => {
    view.current = { from: next.from, end: next.end, width: next.width };
    popularity.onSummaryView(next);
  }, [popularity.onSummaryView]);
  const [page, setPage] = useChangedState<{ index: typeof index; cluster: MarkCluster; first: number } | undefined>(undefined);
  const shownPage = page?.index === index ? page : undefined;
  const onMarkHover = useCallback((cluster: MarkCluster | null) => {
    if (!cluster) return;
    setPage(previous => previous?.index === index && previous.cluster.firstOrdinal === cluster.firstOrdinal && previous.cluster.count === cluster.count
      ? previous : { index, cluster, first: Number(cluster.firstOrdinal) });
  }, [index]);
  const [keyboard, setKeyboard] = useState<{ index: typeof index; hour: number | null; revision: number }>();
  const navigation = useRef<AbortController | null>(null);
  useEffect(() => () => navigation.current?.abort(), [index, active]);
  const onNavigateHour = useCallback((key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: readonly [number, number], until: number | null) => {
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    const bounds: [number, number] = [x[0], Math.min(x[1], until ?? x[1])];
    const local = stepHour(markHours, key, from, bounds);
    const land = (hour: number | null) => {
      if (controller.signal.aborted) return;
      setHover(hour);
      setKeyboard(previous => ({ index, hour, revision: (previous?.revision ?? 0) + 1 }));
    };
    if (!track || !keys.length) { land(local); return; }
    void loadTransientProduct<{ hour: unknown }>(runtime.client, { sourceId: track.source.id, revision: track.source.revision, product: 'series-next-change',
      params: { branchId: track.branch.id, family: POPULARITY_FAMILY, lines: keys.map(({ tag, part }) => ({ country: countryFromKey(tag), part })), key, from,
        firstHour: bounds[0], lastHour: bounds[1] } }, controller.signal).then(reply => {
      const remote = typeof reply.hour === 'number' && Number.isSafeInteger(reply.hour) ? reply.hour : null;
      const forward = key === 'Home' || key === 'ArrowRight';
      const candidates = [local, remote].filter((hour): hour is number => hour !== null && (from === null || hour !== from));
      land(candidates.length ? (forward ? Math.min(...candidates) : Math.max(...candidates)) : null);
    }).catch(() => land(local));
  }, [markHours, index, track, keys, runtime.client]);
  const headValue = (band: Band | undefined, hour: number): string | null | undefined => {
    if (!band || !data) return undefined;
    const followed = popularity.followed;
    if (followed) {
      for (const key of band.keys) {
        const row = followed.row(key);
        const value = row < 0 ? null : rowAmount(followed.entry.answer, row, popularity.decimals);
        if (value != null) return formatShare(value);
      }
      return null;
    }
    const raw = rawAt(band, popularity.lines, Math.floor(hour));
    return raw == null ? raw : formatPopularity(raw, data.decimals);
  };
  const detail = data ? [leaderShown(data, leaderAt(data, head)).ideology, partyNameAt(data, head)].filter(Boolean).join(' · ') : '';
  const latest = useRef({ data, list, headValue, detail }); latest.current = { data, list, headValue, detail };
  const followedHead = popularity.followed;
  const share = useMemo(() => liveTexts(runtime.clock, () => live ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) ?? head : head, hour => {
    const { data, list, headValue, detail } = latest.current;
    const band = data ? list.find(item => item.token === rulingAt(data, hour)) : undefined;
    const value = headValue(band, hour);
    return new Map([['share', detail ? separated(value) : value ?? '']]);
  }, {
    version: () => [latest.current, followedHead?.entry.version],
    subscribe: changed => {
      if (!followedHead) return () => {};
      return followedHead.entry.subscribe(changed);
    },
  }), [runtime.clock, live, track, head, JSON.stringify(widget.time), followedHead?.entry]);
  const [peopleWarnings, setPeopleWarnings] = useState<string[]>([]);
  const menu = useContextMenu('Chart actions');
  useEffect(() => {
    const warnings = [...peopleWarnings, ...(data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, peopleWarnings, onDiagnostics]);

  const drawn = useRef({ list, summaryOf: popularity.summaryOf }); drawn.current = { list, summaryOf: popularity.summaryOf };
  const layers = useMemo(() => { const summaryOfBand = bandSummaries(); return [tiledStackLayer(() => {
    const { list, summaryOf } = drawn.current;
    const summaries = list.map(band => summaryOfBand(band, summaryOf));
    return summaries.every(summary => summary !== undefined) ? summaries as NonNullable<typeof summaries[number]>[] : null;
  }, () => drawn.current.list.map(band => band.color), () => [0, 100])]; }, []);
  const chartData = useMemo<ChartData>(() => ({ revision: () => [popularity.revision(), drawn.current.list], subscribe: popularity.subscribe }),
    [popularity.revision, popularity.subscribe]);

  if (!track || !tag) return <WidgetState kind="empty" icon="palette" message="Load a campaign and choose a country to see its politics." />;
  if (following && shownHour === null && (live || window)) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading parties…" />;
  if (!list.length && !data.leaders.length) return <WidgetState kind="unavailable" message="Party data is unavailable." />;

  const ruling = rulingAt(data, head);
  const countryName = keyNameAt(slots, tag, head);
  const rulingBand = list.find(band => band.token === ruling);
  const leaderRun = leaderAt(data, head);
  const leader = leaderShown(data, leaderRun);
  const header = <div className="ruling-party-leader">
    <span className="ruling-party-portrait" style={rulingBand ? { borderColor: rulingBand.color } : undefined}>
      {leader.portrait ? <img src={leader.portrait} alt="" /> : <ArtFallback icon="helmet" />}
    </span>
    <div className="ruling-party-leader-text">
      <strong className={leader.known ? undefined : 'is-notice'}>{leader.name}</strong>
      <span data-tooltip={contentTitleHint(leaderRun?.ideology)}>{detail}{rulingBand && <LiveText live={share} at="share" />}</span>
    </div>
  </div>;
  if (!list.length) return <div className="ruling-party" ref={root}>{header}
    <WidgetState kind="unavailable" message="Party popularity is unavailable." />
  </div>;
  const pageEvents = shownPage ? shownPage.index.page(shownPage.first, Math.min(EVENT_PAGE, Number(shownPage.cluster.count) - (shownPage.first - Number(shownPage.cluster.firstOrdinal)))) : [];
  const clusterGovernments = shownPage ? visibleGovernmentChanges(governments, detailsThrough)
    .filter(row => row.hour >= shownPage.cluster.firstHour && row.hour <= shownPage.cluster.lastHour) : [];
  return <div className="ruling-party" ref={root}>
    {header}
    <CountryPeople runtime={runtime} widget={widget} track={track} tag={tag} hoverHour={hover} active={active} view="politics" onWarnings={setPeopleWarnings} />
    { }
    <Legend label="Parties" items={list.map(band => {
      const value = headValue(band, head);
      return { id: band.token, label: band.name, color: band.color, spokenValue: value === undefined ? 'not read yet' : value ?? 'not recorded' };
    })} />
    <div className="ruling-party-chart">
      <TimeChart label={countryName === null ? 'Party popularity' : `Party popularity in ${countryName}`} extent={extent} playhead={playhead} liveHead={liveHead} data={chartData}
        playback={(options.playback as PlaybackMode) ?? 'draw-in'} afterPlayhead={options.afterPlayhead === true} windowHours={365 * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: 0, yMax: 100, log: false }} layers={layers} active={active} notRecorded={popularity.unrecorded} revision={data}
        onSummaryView={onSummaryView} onHover={setHover} onNavigateHour={onNavigateHour}
        keyboardHour={keyboard?.index === index ? keyboard.hour : undefined} keyboardRevision={keyboard?.index === index ? keyboard.revision : undefined}
        pickOwner={index} markSummaries={markSummaries} onMarkHover={onMarkHover} badgesAtHour={hour => index.at(hour)}
        markTooltip={cluster => ({ title: cluster.firstHour === cluster.lastHour ? dayLabel(cluster.firstHour) : `${dayLabel(cluster.firstHour)} - ${dayLabel(cluster.lastHour)}`,
          rows: [...badgeRows(index.page(Number(cluster.firstOrdinal), Math.min(8, Number(cluster.count)))),
            ...(Number(cluster.count) > 8 ? [{ label: `${Number(cluster.count) - 8} more: pick the mark to list them`, tone: 'muted' as const }] : [])] })}
        formatValue={value => `${Math.round(value)}%`}
        tooltip={(hour, _frame, hovered) => {
          const shown = leaderShown(data, leaderAt(data, hour));
          const name = partyNameAt(data, hour);
          const partyLabel = name === undefined ? null : name ?? 'Not known';
          const rows: TooltipRow[] = [{ label: 'Leader', value: shown.name, tone: shown.known ? undefined : 'muted' as const }];
          if (shown.ideology || partyLabel) rows.push({ label: 'Ideology · party', value: [shown.ideology ?? 'Not recorded', partyLabel ?? 'Not recorded'].join(' · ') });
          const values = list.map(band => ({ band, raw: rawAt(band, popularity.lines, Math.floor(hour)) }));
          if (values.some(value => value.raw === undefined)) rows.push({ label: 'Parties', value: 'Loading…', tone: 'muted' });
          else rows.push(...values.sort((a, b) => Number(b.raw ?? -1) - Number(a.raw ?? -1))
            .map(({ band, raw }) => ({ label: band.token === rulingAt(data, hour) ? `${band.name} (ruling)` : band.name, swatch: band.color,
              value: raw == null ? 'Not recorded' : formatPopularity(raw, data.decimals), tone: raw == null ? 'muted' as const : undefined })));
          if (options.changes) rows.push(...governmentRows(governments, hour, detailsThrough, token => names.get(token) ?? 'Ideology name unavailable'));
          rows.push(...badgeRows(hovered));
          return { title: dayLabel(hour), rows };
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {shownPage && <PartyEventList events={pageEvents} total={Number(shownPage.cluster.count)}
      offset={shownPage.first - Number(shownPage.cluster.firstOrdinal)} pageSize={EVENT_PAGE}
      onClose={() => setPage(undefined)}
      onPrevious={() => setPage({ ...shownPage, first: Math.max(Number(shownPage.cluster.firstOrdinal), shownPage.first - EVENT_PAGE) })}
      onNext={() => setPage({ ...shownPage, first: shownPage.first + EVENT_PAGE })} />}
    {clusterGovernments.length > 0 && <GovernmentChangeList rows={clusterGovernments} name={name} runtime={runtime} trackId={track.id} openMenu={menu.open} />}
    {popularity.error && <WidgetState kind="error" what="party popularity" failure={popularity.error} onRetry={popularity.retry} />}
    {menu.menu}
  </div>;
}
