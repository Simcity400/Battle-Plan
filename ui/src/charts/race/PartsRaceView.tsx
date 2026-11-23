import { useGate, type Gate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { countryKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo,  useRef, useState } from 'react';
import type { ComponentType } from 'react';
import CountryPicker from '../../components/CountryPicker';
import DrillHost from '../drill/DrillHost';
import LiveText, { liveTexts } from '../LiveText';
import ForcePartsBreakdown from './ForcePartsBreakdown';
import type { PartsRaceDetailProps } from './ForcePartsBreakdown';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill } from './partsRaceDrill';
import type { PartsRaceDrillAction, PartsRaceDrillState } from './partsRaceDrill';
import { useDrill } from '../drill/useDrill';
import { countryPartsSelection, pinCountryParts, pinNativeCountryParts, savedCountryPartsSelection } from './countryPartsDrillData';
import WidgetState from '../../components/WidgetState';
import GateState from '../../dashboard/GateState';
import type { IconName } from '../../components/Icons';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { followStore } from '../../dashboard/follow/store.ts';
import { CELL, cellState, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { followRefusal, followsHead, useFollow } from '../../dashboard/follow/useFollow.ts';
import { canonical } from '../../dashboard/follow/canonical.ts';
import { stickyLimit, stickyMembers } from '../../dashboard/follow/stickyMembers.ts';
import type { StickyMembers } from '../../dashboard/follow/stickyMembers';
import { useTiles } from '../../dashboard/tiles/useTiles';
import { unrecordedReason } from '../../dashboard/tiles/view.ts';
import { loadProduct } from '../../dashboard/products';
import { chartSummaryLines, scaleSummary } from '../chartSummary';
import type { ChartSummary } from '../chartSummary';
import { partsRaceAt, partsRaceEntries, partsRaceLineRuns, partsRaceMixBands, partsRaceModel, partsRacePartName,
  partsRaceSegmentAt, partsRaceTooltips, formatPartsRaceValue, partsRaceAllZero, rankPartsRace } from '../../dashboard/partsRace.ts';
import { useFrameWindows } from '../../widgets/army-size/frameWindows.ts';
import { pausedKey, useHeadHour } from '../../widgets/army-size/headHour.ts';
import { liveArmy } from '../../widgets/army-size/liveArmy.ts';
import type { LiveArmy } from '../../widgets/army-size/liveArmy';
import { dayLabel } from '../draw';
import type { Span } from '../series';
import { CHART_LAYOUT } from '../tokens';
import CountryRaceView from './CountryRaceView';
import { linesTooltip } from './lines';
import { mixCountry } from './options';
import type { RaceView } from './options';
import { followedForceEntries, forceRaceEmpty, forceWindows, joinForceWindows, validateForceReply } from './forceRace.ts';
import type { ForceParams } from './forceRace';
import type { TimeChartProps } from '../TimeChart';
import type { ForceSizeReply, MissingSpan } from '../../generated/dashboard';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

import { forwardRequest, type DeclaredProduct } from '../../dashboard/request.ts';
import type { ParamsFor } from '../../dashboard/regionTime';

export interface PartsRaceViewConfig {
  product: DeclaredProduct; title: string; subject: string; icon: IconName;
  details?: ComponentType<PartsRaceDetailProps>;
  options: readonly OptionField[];
  request: (branchId: string, countries: { tags: readonly string[]; top?: number }, options: Readonly<Record<string, OptionValue>>) => ForceParams;
  totals?: (options: Readonly<Record<string, OptionValue>>) => { kind: string; measure: string; decimals: number };
  referenced?: boolean;
  measure: (options: Readonly<Record<string, OptionValue>>) => string;
  ownership: (options: Readonly<Record<string, OptionValue>>) => string;
  partsLabel: (options: Readonly<Record<string, OptionValue>>) => string;
  chartOptions?: (options: Readonly<Record<string, OptionValue>>) => Readonly<Record<string, OptionValue>>;
  zeroMix: (options: Readonly<Record<string, OptionValue>>) => string;
  positiveTop?: boolean;
  formatValue?: (value: number, options: Readonly<Record<string, OptionValue>>) => string;
  formatTick?: (options: Readonly<Record<string, OptionValue>>) => ((value: number, ticks: readonly number[]) => string) | undefined;
  referenceLabel?: (options: Readonly<Record<string, OptionValue>>) => string | undefined;
  note?: (options: Readonly<Record<string, OptionValue>>) => string;
}

const NO_WARNINGS: string[] = [];

function followedRows(answer: FollowedAnswer | undefined): string {
  if (!answer || answer.kind === 'notRecorded') return answer?.kind ?? '';
  const names: string[] = [];
  for (let row = 0; row < answer.rows; row++) if (cellState(answer, row * answer.columns) === CELL.value) names.push(rowName(answer, row) ?? String(row));
  return names.sort().join(',');
}

function useRaceLines(config: PartsRaceViewConfig, options: Readonly<Record<string, OptionValue>>, runtime: WidgetProps['runtime'], gate: Pick<Gate, 'span'>,
  track: ReturnType<WidgetProps['runtime']['tracks']['find']>, params: ForceParams | null, active: boolean) {
  const totals = config.totals?.(options) ?? null;
  const ranked = params?.top != null;
  const tiles = useTiles(runtime.client, track && params && totals && !ranked ? { source: track.source, span: gate.span, branchId: params.branchId,
    family: { kind: totals.kind, measure: totals.measure }, tags: params.countries.map(countryKey), lastHour: track.branch.lastHour } : null, active);
  const [view, setView] = useState<{ key: string; from: number; to: number; width: number }>();
  const [ranking, setRanking] = useState<{ key: string; lines: Map<string, ChartSummary>; unrecorded: string | null }>();
  const key = JSON.stringify([track?.source.id, track?.source.revision, params, totals]);
  const factor = 10 ** (totals?.decimals ?? 0);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from || !totals) return;
    if (!ranked) { tiles.onView(next); return; }
    setView(before => before?.key === key && before.from === next.from && before.to === next.to && before.width === next.width ? before
      : { key, from: next.from, to: next.to, width: next.width });
  }, [key, ranked, tiles.onView]);
  const viewKey = view?.key === key ? JSON.stringify([view.from, view.to, view.width]) : null;
  const latest = useRef({ view, params, track, gate }); latest.current = { view, params, track, gate };
  useEffect(() => {
    const { view: shown, params: asked, track: bound, gate: held } = latest.current;
    const range = shown ? held.span(shown.from, shown.to) : null;
    if (!active || !ranked || !totals || !bound || !asked || !shown || !viewKey || !range) return;
    const controller = new AbortController();
    const { branchId, top } = asked;
    void loadProduct(runtime.client, productRequest({ sourceId: bound.source.id, revision: bound.source.revision, product: 'series-summary',
      params: { branchId, family: { kind: totals.kind, measure: totals.measure }, top, from: range.firstHour, to: range.lastHour, width: shown.width } }),
    controller.signal, (value: unknown) => {
      const lines = chartSummaryLines<{ tag: string; summary: ChartSummary; missing: MissingSpan[] }>((value as { lines?: unknown } | null)?.lines,
        line => typeof line.tag === 'string' && Array.isArray(line.missing));
      if (!lines) throw Error('The force summary is malformed.');
      return lines;
    }, bound.source).then(found => { if (!controller.signal.aborted) setRanking({ key, lines: new Map(found.map(line => [line.tag, scaleSummary(line.summary, factor)])), unrecorded: unrecordedReason(found) }); }, () => {});
    return () => controller.abort();
  }, [active, ranked, key, viewKey, factor, runtime.client, totals?.kind, totals?.measure, gate.span]);
  const summaries = useMemo(() => ranked ? ranking?.key === key ? ranking.lines : new Map<string, ChartSummary>()
    : new Map([...(tiles.lines?.values() ?? [])].map(line => [line.tag, scaleSummary(line.summary, factor)])), [ranked, ranking, key, tiles.lines, factor]);
  const unrecorded = ranked ? ranking?.key === key ? ranking.unrecorded : null : tiles.unrecorded;
  return { summaries, onSummaryView, error: ranked ? null : tiles.error, unrecorded };
}

const HOLDERS = { holders: true };

export default function PartsRaceView({ widget, runtime, active, countrySlot = null, onDiagnostics, config }: WidgetProps & { config: PartsRaceViewConfig }) {
  const options = useMemo(() => resolveOptions(config.options, widget.options), [config.options, widget.options]);
  const chartOptions = useMemo(() => config.chartOptions?.(options) ?? options, [config, options]);
  const pinned = options.show === 'breakdown';
  const view = (pinned ? 'mix' : options.view ?? 'race') as RaceView;
  const split = options.split === true;
  const formatValue = useCallback((value: number) => config.formatValue?.(value, options) ?? formatPartsRaceValue(value), [config, options]);
  const liveRef = useRef<LiveArmy | null>(null);
  const missingKey = useRef<(hour: number) => string>(() => '');
  const followedAnswer = useRef<(() => string) | null>(null);
  const playbackHour = useHeadHour(runtime.clock, active && widget.time.mode === 'follow', hour =>
    pausedKey(view === 'race' ? followedAnswer.current?.() ?? liveRef.current?.structure() ?? missingKey.current(hour) : missingKey.current(hour), hour,
      runtime.clock.getSnapshot().playing));
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active);
useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const axisOffset = track?.axisOffset ?? 0;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const headAt = (hour: number) => {
    const at = track && resolveWindow(widget, track, runtime.tracks, hour);
    return at && following ? at.lastHour : null;
  };
  const momentAt = (hour: number) => Math.floor(headAt(hour) ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, lastHour]);
  const params = track && (resolved.top || resolved.tags.length) ? config.request(track.branch.id, resolved, options) : null;
  const validate = useCallback((value: unknown) => validateForceReply(value, config.referenced), [config.referenced]);
  const manifest = useProduct<ForceSizeReply>(runtime.client, track && params ? forwardRequest(track.source,
    config.product, { ...params, window: null } as ParamsFor<DeclaredProduct>) : null, active, validate, track?.source);
  const identity = track && params && manifest.data ? JSON.stringify([track.source.id, track.source.revision, config.product, params]) : null;
  const viewport = useRef<{ key: string; range: Span } | null>(null);
  const viewportKey = JSON.stringify([identity, view, extent, options.playback, options.windowDays, options.afterPlayhead]);
  const followedRace = useRef(false);
  const tipHour = useRef<number | null>(null);
  const windowsAt = (hour: number) => {
    if (followedRace.current) {
      const tip = tipHour.current;
      return tip === null ? [] : forceWindows(manifest.data?.windows ?? [], { firstHour: tip, lastHour: tip }, tip, false);
    }
    const at = momentAt(hour);
    const seen = view !== 'race' && viewport.current?.key === viewportKey ? viewport.current.range : { firstHour: at, lastHour: at };
    return forceWindows(manifest.data?.windows ?? [], seen, at, following && view === 'race');
  };
  const subscribeClock = useMemo(() => (changed: () => void) => {
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const history = useFrameWindows<ForceSizeReply>(runtime.client, identity, index => (forwardRequest(track!.source,
    config.product, { ...params!, window: index } as ParamsFor<DeclaredProduct>)), () => windowsAt(runtime.clock.getHour()), subscribeClock, active && identity !== null,
  validate, track?.source);
  missingKey.current = hour => windowsAt(hour).filter(index => !history.holds(index)).join(',');
  const onViewChange = useCallback((x: readonly [number, number]) => {
    viewport.current = { key: viewportKey, range: { firstHour: Math.floor(x[0]), lastHour: Math.ceil(x[1]) } };
    history.check();
  }, [viewportKey, history]);
  const reply = useMemo(() => manifest.data ? joinForceWindows(manifest.data, history.data) : undefined, [manifest.data, history.data]);
  const model = useMemo(() => reply ? partsRaceModel(reply) : null, [reply]);
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  const chosen = widget.countries.kind === 'tags' || widget.countries.kind === 'selected';
  const candidates = (hour: number) => model ? chosen ? model.tags : rankedCandidates(resolved, hour, model.tags) : [];
  const positive = config.positiveTop === true && resolved.top != null;
  const rank = (hour: number) => model ? rankPartsRace(model, candidates(hour), hour, slots, positive) : [];
  const ranked = useMemo(() => rank(moment), [model, resolved, moment, slots, positive]);
  const entries = useMemo(() => model ? partsRaceEntries(model, ranked, split, formatValue) : [], [model, ranked, split, formatValue]);
  const liveRaceView = active && widget.time.mode === 'follow' && view === 'race';
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => { if (anchor.current !== rootElement) setRoot(anchor.current); });
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const followQuery = useMemo(() => liveRaceView && params && !split ? gate.follow({ product: config.product, params }) : null,
    [liveRaceView, JSON.stringify(params), split, config.product, gate.follow]);
  const sticky = useRef<StickyMembers | null>(null);
  const followKey = followQuery ? canonical(followQuery) : '';
  const top = resolved.top ?? null;
  const rowsShape = useCallback((answer: FollowedAnswer | undefined) => {
    if (top == null || !answer || answer.kind === 'notRecorded') return followedRows(answer);
    const names: string[] = [];
    for (let row = 0; row < answer.rows; row++) if (cellState(answer, row * answer.columns) === CELL.value) names.push(rowName(answer, row) ?? String(row));
    sticky.current = stickyMembers(sticky.current, followKey, names, stickyLimit(top));
    return sticky.current.tags.join(',');
  }, [top, followKey]);
  const followed = useFollow(store, followQuery, root, () => liveRef.current?.wake(), rowsShape);
  const followedHead = followsHead(followed) && followed.entry !== null;
  const refused = followRefusal(followed);
  followedRace.current = followedHead && view === 'race';
  const followedNow = followedRace.current ? followed.entry : null;
  followedAnswer.current = followedNow ? () => rowsShape(followedNow.answer) : null;
  const live = useMemo(() => {
    if (!liveRaceView) return null;
    if (followedHead && followed.entry) {
      const entry = followed.entry;
      return liveArmy(runtime.clock, axisOffset, hour => followedForceEntries(entry.answer, resolved.top != null ? null : resolved.tags, resolved, hour, slots, formatValue),
        () => entry.version, undefined, resolved.membership != null);
    }
    return model ? liveArmy(runtime.clock, axisOffset, hour => partsRaceEntries(model, rank(hour), split, formatValue)
      .map(entry => ({ ...entry, hour })), () => 0, hour => missingKey.current(hour)) : null;
  }, [liveRaceView, followedHead, followed.entry, runtime.clock, axisOffset, resolved, slots, model, split, formatValue]);
  liveRef.current = live;
  const lines = useRaceLines(config, options, runtime, gate, track, params, active && view === 'lines');
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]);
  const cutTo = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;
  const lineTags = useMemo(() => [...new Set([...(model?.tags ?? []), ...lines.summaries.keys()])], [model, lines.summaries]);
  const drawnLines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = partsRaceLineRuns(model, tag, cutTo(tag), range);
    return runs.length || lines.summaries.has(tag) ? [{ tag, runs }] : [];
  }), [model, view, lineTags, range, lines.summaries, resolved.membership]);
  const parts = useMemo(() => (reply?.parts ?? []).map(part => ({ id: part.key, label: partsRacePartName(part) })), [reply]);
  const mixTag = mixCountry(widget.countries, fallback, ranked) ?? ranked[0]?.tag ?? null;
  const bands = useMemo(() => model && view === 'mix' && mixTag ? partsRaceMixBands(model, mixTag, cutTo(mixTag), range) : null,
    [model, view, mixTag, range, resolved.membership]);
  const mix = useMemo(() => {
    if (!model || !mixTag || !bands) return null;
    const at = isMember(cutTo(mixTag) ?? undefined, moment) || !resolved.membership ? partsRaceAt(model, mixTag, moment) : null;
    return { tag: mixTag, bands, compactStack: true, parts: parts.map((part, index) => ({ ...part,
      value: at?.values[index] == null ? 'Not known' : formatValue(at.values[index]!) })) };
  }, [model, mixTag, bands, moment, parts, formatValue]);
  const warnings = reply?.warnings ?? manifest.data?.warnings;
  useLayoutEffect(() => {
    onDiagnostics(warnings?.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, onDiagnostics]);
  const pending = !(followedHead && view === 'race') && windowsAt(playbackHour).some(index => !history.data.has(index));
  const failure = manifest.error ?? windowsAt(playbackHour).map(index => history.errors.get(index)).find(Boolean) ?? null;
  const measure = config.measure(options), ownership = config.ownership(options), partsLabel = config.partsLabel(options);
  const referenceLabel = config.referenceLabel?.(options);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon={config.icon} message={`Load a campaign to see its ${config.subject}.`} />;
    if (refused) return <WidgetState kind="unavailable" message={refused} />;
    if (gate.outside) return <GateState gate={gate} loading={`Loading ${config.subject}…`} />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (failure) return <WidgetState kind="error" failure={failure} onRetry={() => { manifest.retry(); history.retry(); }} />;
    if (grouped && groups.error && !groups.data) return <WidgetState kind="error" what="country groups" failure={groups.error} onRetry={groups.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon={config.icon} message={resolved.reason} />;
    if (followedHead) return followed.entry?.answer === undefined ? <WidgetState kind="loading" label={`Loading ${config.subject}…`} /> : undefined;
    if (!model || (grouped && !groups.data)) return <WidgetState kind="loading" label={`Loading ${config.subject}…`} />;
    if (view === 'race' && positive && !ranked.length && !pending && partsRaceAllZero(model, candidates(moment), moment)) {
      return <WidgetState kind="empty" icon={config.icon} message={config.zeroMix(options)} />;
    }
    if (view === 'mix' && mixTag && !parts.length && model.series.get(mixTag)?.series.runs.some(run => run.total === 0)) {
      return <WidgetState kind="empty" icon={config.icon} message={config.zeroMix(options)} />;
    }
    return undefined;
  })();
  const shownEntries = live?.entries();
  const shownTags = view === 'race' ? [...new Set([...(shownEntries ?? ranked).map(entry => entry.tag), ...(followedNow && top != null ? sticky.current?.tags ?? [] : [])])]
    : view === 'lines' ? lineTags : mixTag ? [mixTag] : [];
  const drillControl = useDrill<PartsRaceDrillState, PartsRaceDrillAction>(runtime, useRegion(widget), pinned ? 'breakdown' : null, reducePartsRaceDrill, CLOSED_PARTS_RACE, HOLDERS);
  const drill = drillControl.state, dispatchDrill = drillControl.dispatch;
  const [origin, setOrigin] = useState<{ tag: string; hour: number } | null>(null);
  const resetKey = JSON.stringify([track?.source.id, track?.branch.id, config.product, params, options.show, widget.options?.holder, widget.options?.drillOriginHour]);
  useEffect(() => { dispatchDrill({ type: 'close' }); setOrigin(null); }, [resetKey]);
  const restored = pinned ? savedCountryPartsSelection(widget, track) : undefined;
  const clockHour = track ? Math.floor(runtime.clock.getHour()) + axisOffset : null;
  const choices = drill.choices?.key === resetKey ? drill.choices : null;
  const drillOpen = choices !== null || drillControl.open;
  const pinnedTag = restored?.tag ?? (widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : mixTag);
  const breakdownTag = pinned ? restored === null ? null : pinnedTag : drill.breakdown;
  const savedOrigin = typeof widget.options?.drillOriginHour === 'number' ? widget.options.drillOriginHour : null;
  const breakdownFollows = drillControl.following;
  const drillHour = choices !== null ? drill.hour : breakdownFollows ? clockHour : drillControl.hour;
  const drillAt = Math.floor(drillOpen ? drillHour ?? moment : moment);
  const originHour = pinned ? restored?.hour ?? savedOrigin ?? moment : origin !== null && origin.tag === breakdownTag ? origin.hour : drillAt;
  const selection = breakdownTag && reply ? countryPartsSelection(reply.series, breakdownTag, originHour) : null;
  const openAt = (tag: string, hour: number) => {
    setOrigin({ tag, hour: Math.floor(hour) });
    drillControl.show(tag, hour);
  };
  const open = (hour: number, tag?: string, choices?: readonly string[]) => {
    if (!active) return;
    const actors = partsRaceDrillActors(tag, choices, () => model ? mixCountry(widget.countries, fallback, rank(hour)) : null);
    if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour });
    else if (actors.length) openAt(actors[0], hour);
    else if (choices !== undefined) dispatchDrill({ type: 'close' });
  };
  const dayLive = useMemo(() => liveTexts(runtime.clock, () => Math.floor(runtime.clock.getHour()) + axisOffset, hour => new Map([['day', dayLabel(hour)]])),
    [runtime.clock, axisOffset]);
  const breakdown = <ForcePartsBreakdown runtime={runtime} widget={widget} track={track} active={active} options={chartOptions} product={config.product}
    request={track && breakdownTag ? config.request(track.branch.id, { tags: [breakdownTag] }, options) : null} manifest={manifest.data} validate={validate}
    extent={pinned ? extent : track ? [track.branch.firstHour, track.branch.lastHour] : extent}
    playhead={breakdownFollows ? clockHour : drillAt} tag={breakdownTag} originHour={originHour}
    holder={pinned ? restored?.key ?? undefined : selection?.key ?? undefined} hour={drillAt} following={breakdownFollows} pinned={pinned}
    countrySlot={pinned ? countrySlot : null} saved={!pinned || restored !== null}
    onPick={hour => { if (active) dispatchDrill({ type: 'pick', hour }); }}
    picker={!pinned && model ? { tags: model.tags, onChange: (tag, hour) => openAt(tag, hour) } : null} details={config.details}
    measure={measure} ownership={ownership} partsLabel={partsLabel} referenceLabel={referenceLabel} note={config.note?.(options)} formatValue={formatValue} />;
  if (pinned) return <div className="parts-race" ref={anchor}><DrillHost drill={drillControl} runtime={runtime} mode="inline" anchor={anchor} title={partsLabel}
    day={breakdownFollows && track ? <LiveText live={dayLive} at="day" /> : dayLabel(drillAt)} canFollow={track !== undefined} main={null}>
    {restored === null ? <WidgetState kind="unavailable" message="Saved country unavailable." /> : state && !model ? state : breakdown}</DrillHost></div>;
  const choice = choices && track ? <div className="parts-race-details">
    <p className="drill-empty">Choose a country for {dayLabel(drillAt)}.</p>
    <CountryPicker countries={track.source.metadata.countries} allowedTags={choices.tags} selectedTag={null} playerCountryId={null}
      source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour: drillAt }}
      onChange={tag => {
        if (!active || tag === null || !choices.tags.includes(tag)) return;
        setOrigin({ tag, hour: drillAt });
        dispatchDrill({ type: 'choose', key: choices.key, tag, hour: drillAt });
      }} />
  </div> : breakdown;
  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  const pinHeld = drillControl.gate.checkHold(breakdownTag ? { tag: selection?.tag ?? breakdownTag, origin: selection?.hour ?? originHour } : null);
  const pinCountry = pinHeld.held;
  return <div className="parts-race" ref={anchor}>
    <DrillHost drill={{ ...drillControl, open: drillOpen }} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={partsLabel} day={breakdownFollows && track ? <LiveText live={dayLive} at="day" /> : dayLabel(drillAt)}
      canFollow={choices === null && track !== undefined}
      returnFocus=".race-chart canvas, .time-chart canvas"
      pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={track && pinCountry && pinTime ? () => runtime.addWidgetBelow(widget.id, selection
        ? pinNativeCountryParts(widget, track, pinCountry, pinTime, partsLabel)
        : pinCountryParts(widget, track, pinCountry, pinTime, partsLabel)) : undefined}
      main={<CountryRaceView runtime={runtime} track={track} state={state} active={active && (!drillOpen || options.drill === 'popout')} onPick={open}
      options={chartOptions} view={view} extent={extent}
      playhead={playhead} moment={moment} slots={slots} ranked={shownEntries ?? entries} liveRanked={live ?? undefined} lines={drawnLines}
      parts={parts} mix={mix} countryTags={shownTags} label={config.title} formatValue={formatValue} formatTick={config.formatTick?.(options)}
      segmentAt={(tag, hour) => partsRaceSegmentAt(model, tag, hour)}
      empty={forceRaceEmpty(config.subject, dayLabel(moment), followedHead ? followed.entry?.answer : undefined)}
      onViewChange={view === 'race' ? undefined : onViewChange}
      lineSummaries={lines.summaries.size ? lines.summaries : undefined} onSummaryView={view === 'lines' ? lines.onSummaryView : undefined}
      notRecorded={view === 'lines' && drawnLines.every(line => !line.runs.length) ? lines.unrecorded : null}
      beforeChart={pending ? <WidgetState kind="loading" label={`Updating ${config.subject}…`} /> : undefined}
      raceLegendParts={split ? parts : []} partsLabel={partsLabel}
      labels={{ race: hour => `${config.title}, ${measure}, ${dayLabel(hour)}`, lines: `${config.title} over time, ${measure}`,
        mix: name => `${config.title} of ${name}, ${partsLabel.toLowerCase()}`, actions: `${config.title} actions` }}
      resetKey={JSON.stringify([track?.source.id, track?.branch.id, config.product, params])}
      presentation={{ exact: false, held: false, lineColourHours: 'labels', tooltipKeep: 1, linePadding: CHART_LAYOUT.countryEndLabels,
        strongMixName: true, windowedLegend: true, mixKeyHours: 'starts', raceEmptyState: false, resetTimeView: true }}
      lineEmpty={{ icon: config.icon, message: 'No countries to draw. Choose countries in the widget settings.' }}
      mixEmpty={{ icon: config.icon, message: 'Choose a country in the widget settings.', unavailable: name => `Breakdown unavailable for ${name}.` }}
      tooltip={(tag, hour, name, colours) => {
        if (followedRace.current && tipHour.current !== Math.floor(hour)) { tipHour.current = Math.floor(hour); history.check(); }
        if (!model) return null;
        const at = isMember(cutTo(tag) ?? undefined, hour) || !resolved.membership ? partsRaceAt(model, tag, hour) : null;
        return at ? partsRaceTooltips({ model, name, at, date: dayLabel(hour), measure, ownership, colours, note: config.note?.(options), formatValue, referenceLabel }) : null;
      }}
      lineTooltip={(hour, _tags, labelAt, country) => [linesTooltip(hour, drawnLines, labelAt, country, dayLabel(hour), `${measure} · ${ownership}`, formatValue)]} />}>
      {choice}
    </DrillHost>
  </div>;
}
