import { countryFromKey } from '../../dashboard/countryKey.ts';
import { axisAt, OUTSIDE_MESSAGE, regionHour, SPEC, type GatedHour } from '../../dashboard/regionTime.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import GateState from '../../dashboard/GateState';
import { countryKey } from '../../dashboard/countryKey.ts';
import { arsenalRows } from '../../widgets/nuclear-arsenal/nuclearArsenalData.ts';
import type { HeadProduct } from '../../dashboard/headProduct';
import type { NuclearArsenalReply } from '../../generated/dashboard';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { FollowEntry } from '../../dashboard/follow/store';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useKeyTagsAt } from '../../dashboard/useKeyTagsAt.ts';
import type { ReactNode } from 'react';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import WidgetState from '../../components/WidgetState';
import { headState } from '../../dashboard/HeadState';
import type { IconName } from '../../components/Icons';
import CountryRaceView from './CountryRaceView';
import type { CountryRaceEntry, CountryRaceLine, CountryRacePart } from './countryRace';
import type { LabelAt } from './lines';
import type { RaceView } from './options';
import { mixCountry } from './options';
import { dayLabel } from '../draw';
import { CHART_LAYOUT } from '../tokens';
import type { ChartSummary } from '../chartSummary';
import type { TimeChartProps } from '../TimeChart';
import type { TooltipContent, TooltipRow } from '../tooltip';
import DrillHost from '../drill/DrillHost';
import { pinnedBreakdown } from '../drill/drillState';
import { useDrill } from '../drill/useDrill';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { loadProduct, useFollowedProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { followStore } from '../../dashboard/follow/store.ts';
import { useInstalledValue } from '../../dashboard/follow/useFollowed.ts';
import { CELL, cellState, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer, FollowedRows } from '../../dashboard/follow/message';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { followFailure, followRefusal, followsHead, usePlaybackHour, useFollow } from '../../dashboard/follow/useFollow.ts';
import { canonical } from '../../dashboard/follow/canonical.ts';
import { stickyLimit, stickyMembers } from '../../dashboard/follow/stickyMembers.ts';
import type { StickyMembers } from '../../dashboard/follow/stickyMembers';
import { followAtHead } from '../../dashboard/tiles/head.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import { unrecordedReason } from '../../dashboard/tiles/view.ts';
import type { Live } from '../live';
import { SECONDARY_DASH } from '../palette.ts';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import type { SeriesStateReply } from '../../generated/dashboard';
import { useMeasureSeries } from '../../widgets/country-measure/measureSeries';
import { followedRace, followedRowTags, liveRace, raceLookTags, stateEntries, validateRankedLines, validateRankReply } from './measureRaceData.ts';
import type { RankedLine, RankedMeasure } from './measureRaceData';
import { drawnItems, hatchEntry, hatchesOf, keyedIds, splitEntry } from './measureParts.ts';
import type { MeasureParts, PartValues } from './measureParts';
import { PartFollows, PartsHead, usePartStates } from './PartFollows';
import PartsBreakdown from './PartsBreakdown';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const holds = (summary: ChartSummary) => summary.buckets.some(bucket => bucket !== null && bucket.maxValue > 0);

function valuedRows(answer: FollowedRows): string[] {
  const names: string[] = [];
  for (let row = 0; row < answer.rows; row++) if (cellState(answer, row * answer.columns) === CELL.value) names.push(rowName(answer, row) ?? String(row));
  return names;
}

function followedRows(answer: FollowedAnswer | undefined): string {
  if (!answer || answer.kind === 'notRecorded') return answer?.kind ?? '';
  return valuedRows(answer).sort().join(',');
}

const TOP_ROWS = '=';

function useViewReply<V, T>(lane: string | null, view: V | null, viewKey: string, active: boolean, read: (view: V, signal: AbortSignal) => Promise<T>) {
  const [state, setState] = useState<{ lane: string; viewKey: string; data?: T; error?: Failure; errorKey?: string }>();
  const [landed, setLanded] = useState(0), [attempt, setAttempt] = useState(0);
  const flight = useRef<{ lane: string; controller: AbortController } | null>(null);
  const wanted = useRef({ view, viewKey }); wanted.current = { view, viewKey };
  const reader = useRef(read); reader.current = read;
  const shown = useRef(state); shown.current = state;
  useEffect(() => {
    if (!active || lane == null || view == null || flight.current?.lane === lane) return;
    const held = shown.current;
    if (held?.lane === lane && (held.viewKey === viewKey && held.data !== undefined || held.errorKey === viewKey)) return;
    flight.current?.controller.abort();
    const controller = new AbortController(), asked = wanted.current;
    flight.current = { lane, controller };
    reader.current(asked.view as V, controller.signal).then(data => {
      if (!controller.signal.aborted) setState({ lane, viewKey: asked.viewKey, data });
    }, error => {
      if (!controller.signal.aborted) setState(before => ({ ...(before?.lane === lane ? before : { viewKey: asked.viewKey }), lane, error: asFailure(error), errorKey: asked.viewKey }));
    }).finally(() => {
      if (flight.current?.controller !== controller) return;
      flight.current = null;
      if (!controller.signal.aborted) setLanded(count => count + 1);
    });
  }, [lane, viewKey, active, landed, attempt]);
  useEffect(() => () => { if (flight.current?.lane === lane) { flight.current.controller.abort(); flight.current = null; } }, [lane, active]);
  const current = state?.lane === lane ? state : undefined;
  return { data: current?.data, error: current?.errorKey === viewKey ? current.error : undefined,
    retry: () => { setState(undefined); setAttempt(count => count + 1); } };
}

export interface MeasureRaceProps extends Pick<WidgetProps, 'widget' | 'runtime' | 'active' | 'countrySlot'> {
  measure: RankedMeasure;
  decimals: number;
  view: RaceView;
  options: Readonly<Record<string, OptionValue>>;
  label: string; icon: IconName;
  arsenal?: HeadProduct<NuclearArsenalReply>;
  format: (raw: string, decimals: number) => string;
  formatValue: (value: number) => string;
  onPick?: (hour: number, tag?: string) => void;
  children?: ReactNode;
  onWarnings?: (warnings: readonly string[]) => void;
  note?: string;
  parts?: MeasureParts;
  split?: boolean;
  details?: (tag: string, hour: number, following: boolean, at: GatedHour | null) => ReactNode;
  positiveOnly?: boolean;
  breakdown?: boolean;
  second?: RankedMeasure;
  mix?: { tag: string; spans: ReadonlyMap<string, readonly { firstHour: number; lastHour: number }[]> } | null;
  tooltip?: (tag: string, hour: number, name: string, labelAt: LabelAt) => TooltipContent[] | null;
  lineTooltip?: (hour: number, tags: readonly string[], labelAt: LabelAt, country: (tag: string) => string) => Iterable<TooltipContent>;
  onHover?: (hour: number | null) => void;
  beforeChart?: ReactNode;
}

export const secondLineId = (tag: string, measure: RankedMeasure) => JSON.stringify([tag, measure.measure]);

function joinedSummaries(first: Live<ReadonlyMap<string, TiledLine> | null> | null, second: Live<ReadonlyMap<string, TiledLine> | null> | null,
  measure: RankedMeasure | undefined): Live<ReadonlyMap<string, ChartSummary> | null> {
  let held: { a: unknown; b: unknown; out: ReadonlyMap<string, ChartSummary> | null } | null = null;
  return {
    read: () => {
      const a = first?.read() ?? null, b = second?.read() ?? null;
      if (!held || held.a !== a || held.b !== b) held = { a, b, out: !a && !b ? null : new Map([...[...a ?? []].map(([tag, line]) => [tag, line.summary] as const),
        ...[...b ?? []].map(([tag, line]) => [secondLineId(tag, measure!), line.summary] as const)]) };
      return held.out;
    },
    subscribe: changed => { const stop = first?.subscribe(changed), stopSecond = second?.subscribe(changed); return () => { stop?.(); stopSecond?.(); }; },
  };
}

const HOLDERS = { holders: true };

export default function MeasureRace({ arsenal, widget, runtime, active, countrySlot = null, measure, decimals, view, options, label, icon, format, formatValue, onPick, children, onWarnings, note,
  parts, split = false, details, positiveOnly = false, breakdown = true, second: secondMeasure, mix, tooltip: ownTooltip, lineTooltip, onHover, beforeChart }: MeasureRaceProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const fallbackTag = defaultCountryKey(track);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallbackTag ? [fallbackTag] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallbackTag, groups.data, lastHour]);
  const lineMix = view === 'mix' && !parts;
  const mixing = lineMix ? mix ?? null : null;
  const top = lineMix ? null : resolved.top ?? null;
  const mixTags = useMemo(() => mixing ? [...mixing.spans.keys()] : [], [mixing]);
  const tags = lineMix ? mixTags : resolved.tags;
  const slots = top ?? Math.max(1, tags.length);
  const following = widget.time.mode === 'follow';
  const pinnedParts = !!parts && options.show === 'breakdown';
  const savedCountry = options.holder;
  const drillNeeds = useMemo(() => pinnedParts ? { country: { stored: savedCountry, key: 'holder' }, holders: true } : HOLDERS, [pinnedParts, savedCountry]);
  const gate = useGate(runtime, widget, SPEC, active);
  const raceView = pinnedParts || (view === 'mix' && !!parts) ? null : view === 'race' ? 'race' : 'lines';
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const asked = top == null && tags.length > 0;
  const racing = raceView === 'race' || (!!parts && raceView === null && top != null);
  const followQuery = useMemo(() => !arsenal && !pinnedParts && active && following && track && (top != null || asked) && (racing || asked)
    ? gate.follow({ product: 'series-state', params: { branchId: track.branch.id, family: measure, ...(top != null && racing ? { top } : { reads: tags.map(tag => ({ country: countryFromKey(tag) })) }) } }) : null,
  [active, pinnedParts, following, track?.branch.id, measure, top, asked, tags, racing, !!arsenal, gate.follow]);
  const live = useRef<ReturnType<typeof liveRace> | null>(null);
  const sticky = useRef<StickyMembers | null>(null);
  const followKey = followQuery ? canonical(followQuery) : '';
  const ranks = top != null && racing;
  const rowsShape = useCallback((answer: FollowedAnswer | undefined) => {
    if (!ranks || !answer || answer.kind === 'notRecorded') return followedRows(answer);
    sticky.current = stickyMembers(sticky.current, followKey, valuedRows(answer), stickyLimit(top!));
    return TOP_ROWS + sticky.current.tags.join(',');
  }, [ranks, followKey, top]);
  const followed = useFollow(store, followQuery, root, () => live.current?.wake(), rowsShape);
  const followedHead = followsHead(followed) && followed.entry !== null;
  const refused = followRefusal(followed);
  const followErr = followFailure(followed);
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const askHour = following ? null : gate.hour;
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const membership = lineMix ? mixing?.spans ?? null : resolved.membership;
  const member = useMemo(() => membership ? (tag: string, hour: number) => isMember(membership.get(tag), hour) : null, [membership]);
  const shownEntry = useRef<FollowEntry | null>(null);
  useShownEachFrame(runtime.clock, widget.kind === 'nuclear-arsenal' && raceView === 'race' ? null : () => track ? following
    ? shownEntry.current?.through ?? null : moment : null);

  const partsKey = parts ? JSON.stringify(parts.kind === 'keyed' ? [parts.family, parts.partBits] : parts.items.map(item => [item.id, item.measure])) : '';
  const raceHead = useMemo(() => parts ? new PartsHead(parts) : null, [partsKey]);
  const splitting = !!parts && split && raceView === 'race';
  const hatches = useMemo(() => parts?.kind === 'measures' ? hatchesOf(parts.items) : new Map<string, string>(), [parts]);
  const hatching = hatches.size > 0 && raceView === 'race';
  const partsRead = splitting || hatching;
  const entry = racing && followedHead ? followed.entry : null;
  shownEntry.current = entry;
  const followedTags = useMemo(() => !entry ? [] : followed.structure.startsWith(TOP_ROWS)
    ? followed.structure.slice(TOP_ROWS.length).split(',').filter(Boolean) : followedRowTags(entry.answer, top == null ? tags : null),
  [entry, followed.structure, top, tags]);
  const proportional = parts?.kind === 'measures' && parts.proportional === true;
  const partIds = useCallback((values: ReadonlyMap<string, PartValues>) => !parts ? [] : parts.kind === 'measures' ? drawnItems(parts.items).map(item => item.id) : keyedIds(values.values()),
    [parts]);
  const decorate = useCallback((given: readonly CountryRaceEntry[], values: ReadonlyMap<string, PartValues> | undefined) => {
    const entries = positiveOnly && top != null ? given.filter(item => item.value > 0) : given;
    if (!partsRead || !values) return entries;
    if (!splitting) return entries.map(item => hatchEntry(item, hatches, values.get(item.tag)));
    const ids = partIds(values);
    return entries.map(item => splitEntry(item, ids, values.get(item.tag), proportional, hatches));
  }, [partsRead, splitting, hatches, partIds, proportional, positiveOnly, top]);
  const liveRanked = useMemo(() => {
    if (!entry || !track) return null;
    return live.current = liveRace(runtime.clock, axisOffset, hour => decorate(followedRace(entry, top != null ? null : tags, member, hour, slots, decimals, format).entries,
      partsRead && raceHead ? raceHead.values(followedTags) : undefined),
    () => entry.version * 1_000_000 + (raceHead?.version() ?? 0), member !== null, hour => Math.min(hour, entry.through));
  }, [entry, track, runtime.clock, axisOffset, top, tags, member, slots, decimals, format, decorate, partsRead, raceHead, followedTags]);
  useEffect(() => raceHead?.subscribe(() => live.current?.wake()), [raceHead]);
  const rankRequest = !arsenal && racing && track && !following && (top != null || asked) && askHour !== null ? productRequest({
    sourceId: track.source.id, revision: track.source.revision, product: 'series-state',
    params: top != null ? { branchId: track.branch.id, family: measure, hour: askHour, top }
      : { branchId: track.branch.id, family: measure, reads: tags.filter(tag => !member || member(tag, askHour)).map(tag => ({ country: countryFromKey(tag), hour: askHour })) },
  }) : null;
  const ranked = useFollowedProduct<SeriesStateReply>(runtime.client, rankRequest, ['hour', 'reads'], active,
    top != null ? validateRankReply : value => value as SeriesStateReply, track?.source);
  const rankedAt = useMemo(() => {
    if (!ranked.moment) return null;
    const [at, reads] = JSON.parse(ranked.moment) as [unknown, { hour?: unknown }[] | undefined];
    return typeof at === 'number' ? at : typeof reads?.[0]?.hour === 'number' ? reads[0].hour : null;
  }, [ranked.moment]);
  const rankedFits = !!ranked.data && rankedAt !== null && rankedAt === askHour;
  const unsplit = useMemo(() => ranked.data && rankedFits && rankedAt !== null
    ? stateEntries(ranked.data.states.filter(state => !member || member(countryKey(state.country), rankedAt)), ranked.data.decimals, format, rankedAt, slots) : [],
  [ranked.data, rankedFits, member, rankedAt, slots, format]);
  const rankedTags = useMemo(() => unsplit.map(item => item.tag), [unsplit]);
  const askedParts = usePartStates(runtime.client, track?.source, track?.branch.id, parts, rankedTags, askHour, active && partsRead && !following);
  const selection = useMemo(() => ({ top, tags, membership, split: splitting }), [top, tags, membership, splitting]);
  const entries = useMemo(() => arsenal?.data ? arsenalRows(arsenal.data, arsenal.at ?? moment, selection) : decorate(unsplit, askedParts.values),
    [arsenal?.data, arsenal?.at, moment, selection, decorate, unsplit, askedParts.values]);

  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const onReach = useCallback((hour: number | null) => { setHoverHour(hour); onHover?.(hour); }, [onHover]);
  const series = useMeasureSeries(runtime.client, gate, track?.source, track?.branch, measure.kind, measure.measure, raceView === 'lines' && asked ? tags : [],
    following ? null : askHour, gate.recorded(hoverHour), active && raceView === 'lines', false, membership ?? undefined, lastHour, following);
  const lineEntry = raceView === 'lines' && followedHead ? followed.entry : null;
  useEffect(() => {
    if (!lineEntry) return;
    const stops = tags.map((tag, row) => followAtHead(series.head(tag), () => rowAmount(lineEntry.answer, row, decimals)));
    return () => stops.forEach(stop => stop());
  }, [lineEntry, tags, series.head, decimals]);
  const content = useContentRevision(runtime.client, raceView === 'lines' && top != null);
  const [chartView, setChartView] = useState<{ key: string; from: number; to: number; width: number } | null>(null);
  const rankedKey = track && top != null && gate.open ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, measure, top, content]) : null;
  const shownView = chartView?.key === rankedKey ? chartView : null;
  const rankedLines = useViewReply(raceView === 'lines' ? rankedKey : null, shownView, JSON.stringify([shownView?.from, shownView?.to, shownView?.width]), active,
    (at, signal) => {
      const range = gate.span(at.from, at.to);
      return range ? loadProduct(runtime.client, productRequest({ sourceId: track!.source.id, revision: track!.source.revision, product: 'series-summary',
        params: { branchId: track!.branch.id, family: measure, top, from: range.firstHour, to: range.lastHour, width: at.width } }), signal, validateRankedLines, track!.source)
        : Promise.reject(new Error(OUTSIDE_MESSAGE));
    });
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    if (top == null) { series.onSummaryView(next); return; }
    setChartView(before => before?.key === rankedKey && before.from === next.from && before.to === next.to && before.width === next.width ? before
      : { key: rankedKey ?? '', from: next.from, to: next.to, width: next.width });
  }, [top, rankedKey, series.onSummaryView]);
  const lineTags = useMemo(() => top == null ? tags : (rankedLines.data?.lines ?? []).filter(line => !positiveOnly || holds(line.summary)).map(line => line.tag),
    [top, tags, rankedLines.data, positiveOnly]);
  const seconds = useMeasureSeries(runtime.client, gate, track?.source, track?.branch, secondMeasure?.kind ?? measure.kind, secondMeasure?.measure ?? measure.measure,
    raceView === 'lines' && secondMeasure ? lineTags : [], null, null, active && raceView === 'lines' && !!secondMeasure, false,
    top == null ? membership ?? undefined : undefined, lastHour, following);
  const secondQuery = useMemo(() => active && following && track && secondMeasure && raceView === 'lines' && asked
    ? gate.follow({ product: 'series-state', params: { branchId: track.branch.id, family: secondMeasure, reads: tags.map(tag => ({ country: countryFromKey(tag) })) } }) : null,
  [active, following, track?.branch.id, secondMeasure, raceView, asked, tags, gate.follow]);
  const secondFollowed = useFollow(store, secondQuery, root, () => {}, followedRows);
  const secondEntry = secondMeasure && followsHead(secondFollowed) && secondFollowed.entry !== null ? secondFollowed.entry : null;
  useEffect(() => {
    if (!secondEntry) return;
    const stops = tags.map((tag, row) => followAtHead(seconds.head(tag), () => rowAmount(secondEntry.answer, row, decimals)));
    return () => stops.forEach(stop => stop());
  }, [secondEntry, tags, seconds.head, decimals]);
  const summaries = useMemo(() => {
    const out = new Map<string, ChartSummary>();
    if (top == null) for (const [tag, line] of series.held ?? []) out.set(tag, line.summary);
    else for (const line of rankedLines.data?.lines ?? [] as RankedLine[]) if (!positiveOnly || holds(line.summary)) out.set(line.tag, line.summary);
    if (secondMeasure) for (const [tag, line] of seconds.held ?? []) out.set(secondLineId(tag, secondMeasure), line.summary);
    return out;
  }, [top, series.held, rankedLines.data, positiveOnly, secondMeasure, seconds.held]);
  const seriesLive = series.live, secondLive = seconds.live;
  const lineData = useMemo(() => following && (top == null || secondMeasure) ? joinedSummaries(top == null ? seriesLive : null, secondMeasure ? secondLive : null, secondMeasure)
    : undefined, [top, following, seriesLive, secondLive, secondMeasure]);
  const rankedUnrecorded = useMemo(() => unrecordedReason(rankedLines.data?.lines), [rankedLines.data]);
  const firstUnrecorded = raceView !== 'lines' ? null : top == null ? series.notRecorded : rankedUnrecorded;
  const notRecorded = firstUnrecorded && (!secondMeasure || seconds.notRecorded) ? firstUnrecorded : null;
  const lines = useMemo<CountryRaceLine[]>(() => lineTags.flatMap(tag => [{ tag, runs: [] },
    ...(secondMeasure ? [{ tag, id: secondLineId(tag, secondMeasure), runs: [], dash: SECONDARY_DASH }] : [])]), [lineTags, secondMeasure]);

  const resetKey = JSON.stringify([track?.source.id, track?.branch.id, measure, partsKey, widget.countries]);
  const drillControl = useDrill(runtime, useRegion(widget), pinnedParts ? 'breakdown' : null, undefined, undefined, drillNeeds);
  const drill = drillControl.state;
  const drilled = drill.breakdown != null;
  useEffect(() => { drillControl.close(); }, [resetKey]);
  const shownEntries = () => liveRanked ? liveRanked.entries() : entries;
  const mixTag = useInstalledValue(entry ? changed => entry.subscribe(changed) : undefined,
    () => parts && raceView === null ? mixCountry(widget.countries, fallbackTag, shownEntries()) : null);
  const breakdownTag = drilled ? drillControl.gate.country?.tag ?? null : mixTag;
  const breakdownFollows = drilled ? drillControl.following : following;
  const breakdownGate = drilled ? drillControl.gate : gate;
  const breakdownAt = breakdownFollows ? null : breakdownGate.hour;
  const breakdownHour = Math.floor((drilled ? drillControl.hour : breakdownAt) ?? moment);
  const breakdownTags = useMemo(() => breakdownTag ? [breakdownTag] : [], [breakdownTag]);
  const breakdownHead = useMemo(() => parts ? new PartsHead(parts) : null, [partsKey]);
  const breakdownOn = active && !!breakdownTag && breakdownFollows;
  const breakdownAsked = usePartStates(runtime.client, track?.source, track?.branch.id, parts, breakdownTags, breakdownAt, active && !!breakdownTag && !breakdownFollows);
  const [breakdownName] = useKeyTagsAt(runtime, track, breakdownTags, clockHour => breakdownFollows && track
    ? regionHour(breakdownGate.region, track, runtime.tracks, axisAt(clockHour)) ?? breakdownHour : breakdownHour, breakdownFollows, active && !!breakdownTag);
  const anchor = useRef<HTMLDivElement | null>(null);
  const headerWanted = useMemo(() => pinnedParts && countrySlot && breakdownTag ? [{ tag: breakdownTag, hour: breakdownHour }] : [],
    [pinnedParts, countrySlot, breakdownTag, breakdownHour]);
  const headerLabels = useCountryLabelsAt(runtime.client, track, headerWanted, active && pinnedParts && !!countrySlot);
  const headDate = useMemo(() => ({ read: () => Math.floor(runtime.clock.getHour()) + axisOffset, subscribe: runtime.clock.subscribeFrame }), [runtime.clock, axisOffset]);
  const openDrill = (hour: number, tag?: string) => {
    if (!tag) return;
    drillControl.show(tag, hour, { tag, origin: Math.floor(hour) });
  };

  const error = racing && raceView === 'race' ? ranked.error : raceView === 'lines' ? (top == null ? series.error : rankedLines.error) ?? seconds.error : undefined;
  const retry = racing ? ranked.retry : () => { (top == null ? series.retry : rankedLines.retry)(); seconds.retry(); };
  const warnings = useMemo(() => [...(groups.data?.warnings ?? []), ...(groups.error ? [`Country groups could not be loaded: ${groups.error.message}`] : []),
    ...(partsRead && askedParts.error ? [`${label}: its parts could not be read: ${asFailure(askedParts.error).message}`] : [])],
  [groups.data, groups.error, partsRead, askedParts.error, label]);
  useEffect(() => { onWarnings?.(warnings); }, [warnings, onWarnings]);
  const state = (() => {
    const shared = headState(arsenal);
    if (shared) return shared;
    if (!track) return <WidgetState kind="empty" icon={icon} message={`Load a campaign to see ${label.toLowerCase()}.`} />;
    if (refused) return <WidgetState kind="unavailable" technical={refused} />;
    if (followErr) return <WidgetState kind="error" failure={followErr.error} onRetry={followErr.retry} />;
    if (gate.outside) return <GateState gate={gate} loading={`Loading ${label.toLowerCase()}…`} />;
    if (grouped && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon={icon} message={resolved.reason} />;
    if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
    if (entry?.answer?.kind === 'notRecorded') return <WidgetState kind="unavailable" message={entry.answer.reason} />;
    if (grouped && !groups.data) return <WidgetState kind="loading" label={`Loading ${label.toLowerCase()}…`} />;
    if (raceView === 'race' && !entry && !rankedFits && !arsenal?.data) return <WidgetState kind="loading" label={`Loading ${label.toLowerCase()}…`} />;
    return undefined;
  })();
  const partValuesOf = (tag: string): PartValues | undefined => !parts ? undefined
    : entry && raceHead ? raceHead.values(followedTags).get(tag) : askedParts.values?.get(tag);
  const partList = useMemo<CountryRacePart[]>(() => {
    if (!parts) return [];
    if (arsenal?.data && parts.kind === 'keyed') return [...new Set(entries.flatMap(entry => entry.parts?.map(p => p.id) ?? []))].sort().map(id => ({ id, label: parts.name(id) }));
    if (parts.kind === 'measures') return drawnItems(parts.items).map(item => ({ id: item.id, label: item.label }));
    const values = entry && raceHead ? raceHead.values(followedTags) : askedParts.values;
    return values ? keyedIds(values.values()).map(id => ({ id, label: parts.name(id) })) : [];
  }, [parts, entry, raceHead, followedTags, askedParts.values, arsenal?.data, entries]);
  const valueAt = (tag: string): string | null => {
    const shown = shownEntries().find(item => item.tag === tag);
    return shown ? shown.label : null;
  };
  const tooltip = (tag: string, hour: number, name: string, _colours: ReadonlyMap<string, string>, labelAt: LabelAt): TooltipContent[] | null => {
    if (ownTooltip) return ownTooltip(tag, hour, name, labelAt);
    const partRows: TooltipRow[] = splitting && parts ? partList.map((part, index) => {
      const value = partValuesOf(tag)?.get(part.id);
      return { label: part.label, value: value == null ? 'Not known' : formatValue(value), ...(value == null ? { tone: 'muted' as const } : {}), divider: index === 0 };
    }) : [];
    return [{ title: name, subtitle: dayLabel(hour),
      rows: [{ label, value: valueAt(tag) ?? 'Not recorded', tone: valueAt(tag) === null ? 'muted' : undefined, strong: partRows.length > 0 }, ...partRows],
      ...(note ? { note } : {}) }];
  };
  const breakdownOf = (tag: string, picked: boolean) => track && parts ? <><PartsBreakdown runtime={runtime} gate={breakdownGate} at={breakdownAt} track={track} active={active} options={options} parts={parts}
    tag={tag} name={breakdownName ?? 'Country name unavailable'} extent={extent} playhead={drilled ? breakdownHour : playhead} hour={breakdownHour} head={breakdownFollows ? breakdownHead : null}
    values={breakdownAsked.values?.get(tag)} totalLabel={label} format={formatValue} error={breakdownAsked.error} onRetry={breakdownAsked.retry}
    onPick={picked ? drillControl.pick : undefined} />{details?.(tag, breakdownHour, breakdownFollows, breakdownAt)}</> : null;
  const follows = parts && track ? <>
    <PartFollows store={store} gate={gate} branchId={track.branch.id} tags={partsRead ? followedTags : []} on={active && partsRead && !!entry} root={root} head={raceHead!} />
    <PartFollows store={store} gate={breakdownGate} branchId={track.branch.id} tags={breakdownTags} on={breakdownOn} root={root} head={breakdownHead!} />
  </> : null;

  const attachRoot = useCallback((element: HTMLDivElement | null) => { setRoot(element); anchor.current = element; arsenal?.root(element); }, [arsenal?.root]);
  if (view === 'mix' && !pinnedParts) {
    return <div className="parts-race" ref={attachRoot}>{follows}
      {!track ? state : breakdownGate.outside ? <GateState gate={breakdownGate} loading={`Loading ${label.toLowerCase()}…`} />
        : !breakdownTag ? <WidgetState kind="empty" icon={icon} message="Choose a country in the widget settings." /> : breakdownOf(breakdownTag, false)}
      {children}
    </div>;
  }
  const race = (visible: boolean) => <CountryRaceView arsenal={arsenal && { ...selection, exactRead: arsenal.exactRead, offset: axisOffset, follow: arsenal.workerFollow, reply: arsenal.workerFollow === undefined ? arsenal.data : undefined,
      fixed: following ? undefined : moment - axisOffset }} runtime={runtime} track={track} active={active && visible} state={state} options={options}
    view={lineMix ? 'mix' : raceView ?? 'race'} extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={entries} liveRanked={liveRanked ?? undefined} lines={lines}
    countryTags={arsenal ? entries.map(row => row.tag) : raceLookTags(raceView === 'race', liveRanked ? followedTags : null, lineTags)}
    mix={mixing ? { tag: mixing.tag, lines } : null} lineTooltip={lineTooltip} beforeChart={beforeChart}
    label={label} empty={`No ${label.toLowerCase()} recorded on ${dayLabel(moment)}.`} formatValue={formatValue}
    lineSummaries={summaries} lineData={lineData} onSummaryView={raceView === 'lines' ? onSummaryView : undefined} liveHead={liveHead}
    navigation={raceView === 'lines' && top == null ? series : undefined} onReachHour={raceView === 'lines' ? onReach : undefined}
    onPick={parts && breakdown ? openDrill : onPick} parts={partList} raceLegendParts={splitting ? partList : []} partsLabel={parts?.label}
    labels={{ race: hour => `${label}, ${dayLabel(hour)}`, lines: `${label} over time` }}
    lineEmpty={top != null && !rankedLines.data ? undefined : { icon, message: 'No countries to draw. Choose countries in the widget settings.' }} notRecorded={notRecorded}
    resetKey={resetKey}
    presentation={{ exact: true, held: false, lineColourHours: 'labels', windowedLegend: true, linePadding: CHART_LAYOUT.countryEndLabels,
      raceEmptyState: false, resetTimeView: true }}
    tooltip={tooltip} />;
  if (!parts || !breakdown || !track) return <div className="parts-race" ref={attachRoot}>{follows}{pinnedParts ? state : race(true)}{children}</div>;
  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  const pinHeld = drillControl.gate.checkHold(drillControl.gate.country?.holder);
  const pinCountry = pinHeld.held;
  const drillTitle = drill.breakdown && breakdownName ? `${breakdownName}: ${parts.label.toLowerCase()}` : parts.label;
  return <div className="parts-race" ref={attachRoot}>{follows}
    <DrillHost drill={drillControl} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor} title={drillTitle}
      day={dayLabel(breakdownHour)} canFollow
      returnFocus=".country-race-chart canvas"
      pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={drill.breakdown && pinTime && pinCountry ? () => {
        const pinned = pinnedBreakdown(widget, { id: 'breakdown', label: parts.label }, track.id, pinTime);
        runtime.addWidgetBelow(widget.id, { ...pinned, countries: { kind: 'tags', tags: [pinCountry.tag] }, options: { ...pinned.options, holder: pinCountry.token } });
      } : undefined}
      main={pinnedParts ? null : race(drill.breakdown == null || options.drill === 'popout')}>
      {drill.breakdown ? breakdownTag ? breakdownOf(breakdownTag, true) : <GateState gate={drillControl.gate} loading={`Loading ${label.toLowerCase()}…`} /> : null}
    </DrillHost>
    {pinnedParts && countrySlot ? createPortal(<DatedCountryChip label={breakdownTag ? headerLabels.labelAt(breakdownHour)(breakdownTag) : null}
      hour={breakdownHour} live={breakdownFollows ? headDate : undefined} reason={breakdownTag ? null : 'Choose a country in the widget settings.'} />, countrySlot) : null}
    {children}
  </div>;
}
