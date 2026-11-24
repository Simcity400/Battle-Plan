import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import type { IconName } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import { branchHour, lookPlan, looksRead, useCountryLooks } from '../../dashboard/countryLooks';
import { useIdentityHour } from '../../dashboard/atHead';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryCapitulationRows } from '../../dashboard/countryCapitulations';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import type { EventBadge } from '../badges';
import { dayLabel, drawBands, drawLine, drawPlayhead, placeEndLabels } from '../draw';
import { jumpItem } from '../jump';
import Legend from '../Legend';
import { pickLines } from '../linePick';
import { drawApproximateSpans, drawLineSummary, extentOfSummary, pickSummaryLines } from '../chartSummary';
import type { ChartSummary } from '../chartSummary';
import { extentOfAll, lineLabelHours } from '../series';
import type { Run } from '../series';
import TimeChart from '../TimeChart';
import type { LostSpan } from '../gaps';
import { pendingLayer } from '../prepared';
import type { Span } from '../prepared';
import type { ChartData, ChartLayer } from '../TimeChart';
import type { Live } from '../live.ts';
import type { ChartStrip } from '../TimeChart';
import { CHART_LAYOUT } from '../tokens';
import type { ChartTokens } from '../tokens';
import type { PlotFrame } from '../frame';
import { countryNameDetails, fitTooltip, tooltipText } from '../tooltip.ts';
import type { TooltipContent } from '../tooltip';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { shownUntil } from '../view';
import type { PlaybackMode } from '../view';
import { useCountryColours } from './countryColours';
import { countryRaceEnds, countryRaceHours, countryRaceItem, countryRaceItems, countryRacePickTag, countryRaceReadHour, countryRaceStack, countryRaceStackExtent, countryRaceTooltipTags, forEachJoinedCountryRaceSegment, partColours, raceLabelAt } from './countryRace';
import type { CountryRaceEntry, CountryRaceLine, CountryRaceLooks, CountryRacePart, LiveRaceEntries } from './countryRace';
import { linesAt, linesTooltip } from './lines';
import type { LabelAt } from './lines';
import type { RaceView } from './options';
import RaceChart from './RaceChart';
import type { LiveRace, RemoteRaceRows } from './RaceChart';
import type { RaceItem } from './race';
import './countryRace.css';

export interface CountryRaceViewProps {
  runtime: WidgetRuntime; track: BoundTrack | undefined; active: boolean;
  options: Readonly<Record<string, OptionValue>>; view: RaceView;
  extent: [number, number]; playhead: number | null; moment: number; slots: number;
  ranked: readonly CountryRaceEntry[]; lines: readonly CountryRaceLine[];
  liveRanked?: LiveRaceEntries;
  remoteRows?: RemoteRaceRows;
  parts?: readonly CountryRacePart[];
  raceLegendParts?: readonly CountryRacePart[]; partsLabel?: string;
  mix?: { tag: string; tagAt?: (hour: number) => string | null; bands?: readonly (readonly Run[])[]; compactStack?: boolean; lines?: readonly CountryRaceLine[]; parts?: readonly CountryRacePart[]; approximate?: ChartSummary['approximate'] } | null;
  countryTags?: readonly string[];
  gaps?: readonly LostSpan[];
  entityLooks?: CountryRaceLooks;
  lineLegendLabel?: string;
  countryLookHours?: ReadonlyMap<string, number>;
  countryLookHour?: (tag: string | null, hour: number) => number | null;
  segmentAt?: (tag: string, hour: number) => number | null;
  tooltip: (tag: string, hour: number, name: string, colours: ReadonlyMap<string, string>, labelAt: LabelAt) => Iterable<TooltipContent> | null;
  tooltipTags?: (hour: number, tags: readonly string[]) => readonly string[];
  lineTooltip?: (hour: number, tags: readonly string[], labelAt: LabelAt, country: (tag: string) => string,
    colours: ReadonlyMap<string, string>) => Iterable<TooltipContent>;
  timeTooltip?: (hour: number) => Iterable<TooltipContent> | null;
  keyHours?: readonly number[]; strip?: ChartStrip; beforeChart?: ReactNode;
  pending?: readonly Span[];
  badges?: readonly EventBadge[];
  onViewChange?: (x: readonly [number, number]) => void;
  lineSummaries?: ReadonlyMap<string, ChartSummary>;
  lineData?: Live<ReadonlyMap<string, ChartSummary> | null>;
  onSummaryView?: import('../TimeChart').TimeChartProps['onSummaryView'];
  liveHead?: import('../TimeChart').LiveHead;
  onReachHour?: (hour: number | null) => void;
  exactLineTags?: (hour: number) => Promise<readonly string[] | null>;
  navigation?: Pick<import('../TimeChart').TimeChartProps, 'onNavigateHour' | 'keyboardHour' | 'keyboardRevision'>;
  onPick?: (hour: number, tag?: string, choices?: readonly string[]) => void;
  pickOwner?: unknown;
  selectedHour?: number | null;
  selectedHead?: boolean;
  state?: ReactNode;
  formatAxis?: (value: number, ticks: readonly number[]) => string;
  formatTick?: (value: number, ticks: readonly number[]) => string;
  labels?: { race?: string | ((hour: number) => string); lines?: string; mix?: (name: string) => string; actions?: string };
  lineEmpty?: { icon?: IconName; message: string };
  notRecorded?: string | null;
  mixEmpty?: { icon?: IconName; message: string; unavailable: (name: string) => string };
  resetKey?: string;
  presentation?: {
    exact?: boolean; held?: boolean;
    lineColourHours?: 'drawn' | 'labels'; countryLooks?: boolean;
    tooltipKeep?: number; linePadding?: number;
    strongMixName?: boolean; mixKeyHours?: 'boundaries' | 'starts'; raceEmptyState?: boolean;
    resetTimeView?: boolean;
    windowedLegend?: boolean;
    mixLegend?: boolean;
  };
  label: string; empty: string; formatValue: (value: number) => string;
}
const NO_PARTS: readonly CountryRacePart[] = [];
const NO_TAGS: readonly string[] = [];
const NO_HOURS: readonly number[] = [];
const NO_BADGES: readonly EventBadge[] = [];
const COUNTRY_NAME_SAMPLES = ['Country name unavailable'];
type RawPickOwner = { scope: string; client: WidgetRuntime['client'];
  lines: readonly Pick<CountryRaceLine, 'tag' | 'id' | 'runs'>[] | undefined; bands: readonly (readonly Run[])[] | undefined };

export default function CountryRaceView({ runtime, track, active, options, view, extent: shownExtent, playhead, moment, slots, ranked, liveRanked, lines, remoteRows,
  parts = NO_PARTS, raceLegendParts = parts, partsLabel = 'Types', mix, countryTags, countryLookHours, countryLookHour, entityLooks, lineLegendLabel = 'Countries', segmentAt, tooltip, tooltipTags, lineTooltip, timeTooltip,
  keyHours: suppliedHours, strip, beforeChart, pending, badges, onViewChange: reportViewport, lineSummaries, lineData, onSummaryView, liveHead, onReachHour, exactLineTags, navigation, onPick, pickOwner, selectedHour, selectedHead = false, state, formatAxis, formatTick, labels: chartLabels, lineEmpty, notRecorded, mixEmpty, resetKey,
  presentation = {}, label, empty, formatValue, gaps }: CountryRaceViewProps) {
  const [extentFirst, extentLast] = shownExtent;
  const extent = useMemo<[number, number]>(() => [extentFirst, extentLast], [extentFirst, extentLast]);
  const tokens = useChartTokens();
  const hoursPerSecond = usePlaybackMotion(runtime.clock, true);
  const viewport = useViewportHeight();
  const menu = useContextMenu(chartLabels?.actions ?? `${label} actions`);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const endLabels = options.endLabels !== false;
  const drawnLines = view === 'mix' ? mix?.lines ?? [] : lines;
  const linesView = view === 'lines' || (view === 'mix' && mix?.lines !== undefined);
  const [labelView, setLabelView] = useState<[number, number] | null>(null);
  const labelKey = useRef<{ shown: string | null; rendered: () => string; of: (bounds: readonly [number, number]) => string }>({ shown: null, rendered: () => '', of: () => '' });
  const onViewChange = useCallback((x: readonly [number, number]) => {
    const bounds: [number, number] = [Math.floor(x[0]), Math.floor(x[1])];
    const key = labelKey.current.of(bounds);
    if (key === (labelKey.current.shown ??= labelKey.current.rendered())) return;
    labelKey.current.shown = key;
    setLabelView(before => before?.[0] === bounds[0] && before[1] === bounds[1] ? before : bounds);
  }, []);
  const labelUntil = liveHead && playhead != null ? shownUntil(liveHead.read(), playback, options.afterPlayhead === true) : until;
  const labelHours = (bounds: readonly [number, number] | null, last: number | null) => lineLabelHours(drawnLines,
    endLabels ? Math.max(extent[0], bounds?.[0] ?? extent[0]) : extent[0], endLabels ? Math.min(extent[1], bounds?.[1] ?? extent[1]) : extent[1], last);
  const observeView = useMemo(() => {
    const labels = presentation.countryLooks === false || (view === 'mix' && presentation.mixKeyHours === 'starts')
      ? undefined : onViewChange;
    return reportViewport ? (x: readonly [number, number]) => { labels?.(x); reportViewport(x); } : labels;
  }, [presentation.countryLooks, presentation.mixKeyHours, view, onViewChange, reportViewport]);
  const hours = useMemo(() => labelHours(labelView, labelUntil), [drawnLines, endLabels, extent, labelView, labelUntil]);
  const tags = useMemo(() => presentation.countryLooks === false ? NO_TAGS : countryTags
    ?? (view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? [...new Set(lines.map(line => line.tag))] : mix ? [mix.tag] : []),
    [presentation.countryLooks, countryTags, view, ranked, lines, mix]);
  const readHour = useCallback((tag: string, hour: number) => countryRaceReadHour(tag, hour,
    view === 'race' ? countryLookHours : undefined, countryLookHour), [view, countryLookHours, countryLookHour]);
  const lookHoursOf = (at: ReadonlyMap<string, number>) => countryLookHour ? new Map(tags.flatMap(tag => {
    const read = readHour(tag, linesView ? at.get(tag) ?? moment : moment);
    return read === null ? [] : [[tag, read] as const];
  })) : view === 'race' ? countryLookHours : linesView ? at : undefined;
  const lookHours = useMemo(() => lookHoursOf(hours), [countryLookHour, tags, readHour, linesView, hours, moment, view, countryLookHours]);
  const exactLooks = countryLookHour !== undefined || (view === 'race' && countryLookHours !== undefined);
  const followedLooks = active && view === 'race' && !!liveRanked && !exactLooks && presentation.countryLooks !== false && !entityLooks;
  const identityHour = useIdentityHour(runtime.client, track, tags, runtime.clock, followedLooks,
    () => liveRanked && track ? branchHour(track.branch, liveRanked.hour()) : null);
  const lookRead = looksRead(followedLooks, identityHour, moment, segmentAt, lookHours, exactLooks);
  const looks = useCountryLooks(runtime.client, track, tags, lookRead.hour, active, lookRead.segmentAt, lookRead.hours, lookRead.exact);
  const labelsOf = (at: ReadonlyMap<string, number>) => entityLooks ? JSON.stringify([...at])
    : JSON.stringify(lookPlan(track?.branch, tags, moment, segmentAt, lookHoursOf(at), exactLooks).samples);
  labelKey.current.of = bounds => labelsOf(labelHours(bounds, liveHead && playhead != null ? shownUntil(liveHead.read(), playback, options.afterPlayhead === true) : until));
  labelKey.current.rendered = () => labelsOf(hours);
  labelKey.current.shown = null;
  const countryColour = useCountryColours(tags, looks.color, tokens, looks);
  const entity = entityLooks ?? { name: looks.name, nameFailure: looks.nameFailure, flag: looks.flag, color: countryColour,
    countryFlag: presentation.countryLooks === false ? undefined : (tag: string, at?: number) => ({
      nameFailure: looks.nameFailure?.(tag, at), flag: looks.flag(tag, at), flagDetail: looks.flagDetail?.(tag, at), flagReason: looks.flagReason?.(tag, at), flagPending: looks.flagPending?.(tag, at) }) };
  const country = entity.color;
  const nameOf = useCallback((tag: string, hour?: number) => entityLooks
    ? entityLooks.name(tag, hour ?? (linesView ? hours.get(tag) ?? moment : moment)) : looks.name(tag, hour), [entityLooks, looks, linesView, hours, moment]);
  const colours = useMemo(() => partColours(parts, tokens.categories, undefined, tokens.surface), [parts, tokens.categories, tokens.surface]);
  const items = useMemo(() => countryRaceItems(ranked, colours, country, tokens.muted, presentation.exact !== false),
    [ranked, colours, country, tokens.muted, presentation.exact]);
  const raceLabel = useRef(chartLabels?.race);
  raceLabel.current = chartLabels?.race;
  const dateOverlay = options.dateOverlay === true;
  const live = useMemo<LiveRace | undefined>(() => {
    if (!liveRanked) return undefined;
    let source: readonly CountryRaceEntry[] | null = null, made: readonly RaceItem[] = [];
    const itemOf = new WeakMap<CountryRaceEntry, RaceItem>();
    return {
      items: () => {
        const entries = liveRanked.entries();
        if (entries !== source) {
          source = entries;
          made = entries.map(entry => {
            let item = itemOf.get(entry);
            if (!item) itemOf.set(entry, item = countryRaceItem(entry, colours, country, tokens.muted, presentation.exact !== false));
            return item;
          });
        }
        return made;
      },
      label: () => raceLabelAt(raceLabel.current, liveRanked.hour()) ?? `${label}, ${dayLabel(liveRanked.hour())}`,
      overlay: () => dateOverlay ? dayLabel(liveRanked.hour()) : null,
      bind: liveRanked.bind,
      clock: runtime.clock,
    };
  }, [liveRanked, colours, country, tokens.muted, presentation.exact, label, dateOverlay, runtime.clock]);
  const [tip, setTip] = useState<{ hour: number; tags: readonly string[]; events: readonly EventBadge[] } | null>(null);
  const tipKey = useRef('');
  const showTip = (hour: number | null, wanted: readonly string[] = [], events: readonly EventBadge[] = NO_BADGES) => {
    onReachHour?.(hour === null ? null : Math.floor(hour));
    const key = hour == null ? '' : JSON.stringify([hour, wanted, events.map(badge => [badge.hour, badge.capitulationTag])]);
    if (key === tipKey.current) return;
    tipKey.current = key;
    setTip(hour == null ? null : { hour, tags: wanted, events });
  };
  const wanted = useMemo(() => tip ? [...countryRaceTooltipTags(tip.hour, tip.tags, tooltipTags).flatMap(tag => {
    const hour = readHour(tag, tip.hour); return hour === null ? [] : [{ hour, tag }];
  }), ...tip.events.flatMap(badge => badge.capitulationTag === undefined ? [] : [{ hour: badge.hour, tag: badge.capitulationTag }])] : [], [tip, tooltipTags, readHour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  const labelAt: LabelAt = hour => tag => labels.labelAt(readHour(tag, hour) ?? hour)(tag);
  useEffect(() => { if (!active) { tipKey.current = ''; setTip(null); } }, [active]);
  const keyHours = useMemo(() => countryRaceHours(drawnLines), [drawnLines]);
  const lineLayers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const labelHours = presentation.lineColourHours === 'labels'
        ? endLabels ? lineLabelHours(drawnLines, frame.x[0], Math.min(frame.x[1], extent[1]), until) : undefined
        : lineLabelHours(drawnLines, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]), null);
      for (const line of drawnLines) {
        const held = lineData?.read() ?? lineSummaries;
        const color = line.color ?? country(line.tag, labelHours?.get(line.tag)), summary = held?.get(line.id ?? line.tag) ?? held?.get(line.tag);
        if (summary) drawLineSummary(ctx, frame, chartTokens, summary, color, presentation.held !== false, state.playhead, { area: line.area, dash: line.dash, endDot: line.endDot });
        else drawLine(ctx, frame, chartTokens, line.runs, color,
          { held: presentation.held !== false, until: state.playhead ?? undefined, area: line.area, dash: line.dash, endDot: line.endDot });
      }
    },
    extent: x => {
      const held = lineData?.read() ?? lineSummaries;
      if (!held) return extentOfAll(drawnLines.map(line => line.runs), x[0], Math.min(x[1], until ?? x[1]));
      const bounds = drawnLines.flatMap(line => {
        const summary = held.get(line.id ?? line.tag) ?? held.get(line.tag);
        const bounds = summary ? extentOfSummary(summary, x[0], Math.min(x[1], until ?? x[1]))
          : extentOfAll([line.runs], x[0], Math.min(x[1], until ?? x[1]));
        return bounds ? [bounds] : [];
      });
      return bounds.length ? [Math.min(...bounds.map(bound => bound[0])), Math.max(...bounds.map(bound => bound[1]))] : null;
    },
  }], [drawnLines, lineSummaries, lineData, country, until, extent, endLabels, presentation.held, presentation.lineColourHours]);
  const lineChartData = useMemo<ChartData | undefined>(() => lineData && { revision: () => [lineData.read()], subscribe: lineData.subscribe }, [lineData]);
  const mixBands = mix?.bands;
  const stack = useMemo(() => mixBands ? countryRaceStack(mixBands, mix?.compactStack) : null, [mixBands, mix?.compactStack]);
  const mixParts = mix?.parts ?? parts;
  const mixApproximate = mix?.approximate;
  const mixColours = useMemo(() => partColours(mixParts, tokens.categories, colours, tokens.surface), [mixParts, colours, tokens.categories, tokens.surface]);
  const mixHours = useMemo(() => mixBands ? countryRaceHours(mixBands.map((runs, index) => ({ tag: String(index), runs }))) : [], [mixBands]);
  const mixLayers = useMemo<ChartLayer[]>(() => stack ? [{
    draw: (ctx, frame, chartTokens, state) => {
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      const palette = mixParts.map(part => mixColours.get(part.id) ?? chartTokens.muted);
      drawApproximateSpans(ctx, frame, mixApproximate, state.playhead, () => forEachJoinedCountryRaceSegment(stack, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]),
        hoursPerPixel, presentation.held !== false,
        segment => drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, palette, segment)));
    },
    extent: x => countryRaceStackExtent(stack, x[0], Math.min(x[1], until ?? x[1])),
  }] : [], [stack, mixParts, mixColours, mixApproximate, until, presentation.held]);
  const revision = useMemo(() => ({ lines, lineSummaries, mix, selectedHour, looks: looks.revision, labels: labels.revision, entities: entityLooks, viewport }),
    [lines, lineSummaries, mix, selectedHour, looks.revision, labels.revision, entityLooks, viewport]);
  const baseHours = suppliedHours ?? (linesView ? keyHours : presentation.mixKeyHours === 'starts' ? mixHours : stack?.hours ?? NO_HOURS);
  const timeHours = useMemo(() => badges?.length ? [...new Set([...baseHours, ...badges.map(badge => badge.hour)])].sort((a, b) => a - b) : baseHours,
    [baseHours, badges]);
  const rawPickLines = view === 'mix' ? mix?.lines : lines;
  const rawPickBands = view === 'mix' && !mix?.lines ? mixBands : undefined;
  const rawPickScope = JSON.stringify([track?.source.id, track?.branch.id, view, presentation.held !== false,
    view === 'mix' && !mix?.lines ? mix?.tag : undefined]);
  const fallback = useMemo(() => ({ owner: null as RawPickOwner | null }), []);
  const fallbackPickOwner = useMemo(() => {
    if (pickOwner !== undefined) { fallback.owner = null; return undefined; }
    const before = fallback.owner;
    const sameLines = before?.lines && rawPickLines ? before.lines.length === rawPickLines.length
      && before.lines.every((line, index) => line.tag === rawPickLines[index].tag && line.id === rawPickLines[index].id
        && line.runs === rawPickLines[index].runs) : before?.lines === rawPickLines;
    if (before && before.scope === rawPickScope && before.client === runtime.client && before.bands === rawPickBands && sameLines) return before;
    return fallback.owner = { scope: rawPickScope, client: runtime.client, bands: rawPickBands,
      lines: rawPickLines?.map(({ tag, id, runs }) => ({ tag, id, runs })) };
  }, [fallback, pickOwner, rawPickScope, runtime.client, rawPickLines, rawPickBands]);
  const timePickOwner = pickOwner === undefined ? fallbackPickOwner : pickOwner;
  const livePickOwner = useRef(timePickOwner); livePickOwner.current = timePickOwner;
  useEffect(() => () => { livePickOwner.current = null; }, []);
  const mixTagAt = (hour: number) => mix?.tagAt ? mix.tagAt(hour) : mix?.tag ?? null;
  const mixName = mixTagAt(moment) ? nameOf(mixTagAt(moment)!) : 'Selected country unavailable';
  const raceTag = (key: string, item?: RaceItem) => item?.tag ?? key;
  const raceHour = (item?: RaceItem) => liveRanked ? liveRanked.hour() : item?.hour ?? moment;
  const raceCountryFlag = (key: string, item?: RaceItem) => {
    const tag = raceTag(key, item), hour = readHour(tag, raceHour(item));
    return (hour === null ? undefined : entity.countryFlag?.(tag, hour))
      ?? { flagReason: 'The country flag is unavailable at this date.' };
  };
  const timeLayers = useMemo(() => [pendingLayer(pending), ...(linesView ? lineLayers : mixLayers)], [pending, linesView, lineLayers, mixLayers]);
  const timeOverlay = useCallback((ctx: CanvasRenderingContext2D, frame: PlotFrame, chartTokens: ChartTokens) => {
    if (selectedHour != null) drawPlayhead(ctx, frame, chartTokens, selectedHead && liveHead ? Math.floor(liveHead.read()) : selectedHour);
    if (!linesView || !endLabels) return;
    placeEndLabels(ctx, frame, chartTokens, countryRaceEnds(drawnLines, frame.x[0], Math.min(frame.x[1], extent[1]), until,
      nameOf, country, formatValue).map(end => ({ ...end, y: frame.toY(end.value) })));
  }, [selectedHour, selectedHead, liveHead, linesView, endLabels, drawnLines, extent, until, nameOf, country, formatValue]);
  if (state != null || !track) return state ?? null;
  const jump = (hour: number, x: number, y: number, tag: string | null = view === 'mix' ? mixTagAt(hour) : countryRacePickTag(view, drawnLines)) => {
    if (view === 'mix' && mix?.tagAt && tag === null) return;
    const read = tag !== null ? readHour(tag, hour) : countryLookHour ? countryLookHour(null, hour) : hour;
    if (read !== null) menu.open(x, y, [jumpItem(runtime, track.id, read)]);
  };
  const fitForms = (forms: Iterable<TooltipContent>, hovered: readonly EventBadge[], keep = 0) => {
    if (!hovered.length) return fitTooltip(forms, viewport, { keep });
    const rows = countryCapitulationRows(hovered, labels.labelAt);
    function* withEvents() {
      let supplied = false;
      for (const form of forms) {
        supplied = true;
        yield { ...form, rows: [...rows, ...form.rows],
          spoken: form.spoken ? [form.spoken, tooltipText({ title: '', rows })].filter(Boolean).join('; ') : undefined };
      }
      if (!supplied) yield { title: formatDateTime(hovered[0].hour), rows };
    }
    return fitTooltip(withEvents(), viewport, { keep });
  };
  const tipAt = (tag: string, hour: number, name: string, hovered: readonly EventBadge[] = []) => {
    const forms = tooltip(tag, hour, name, colours, labelAt)
      ?? (hovered.length ? [{ title: formatDateTime(hour), rows: [] }] : null);
    if (!forms) return null;
    const detail = countryNameDetails(labelAt(hour)(tag), { nameFailure: entity.nameFailure?.(tag, hour) });
    function* withNameDetails() {
      for (const form of forms!) yield { ...form, ...countryNameDetails({ nameDetail: form.detail }, { nameDetail: detail.detail }) };
    }
    return fitForms(withNameDetails(), hovered, presentation.tooltipKeep ?? 1);
  };
  const common = { extent, playhead, playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24,
    hoursPerSecond, branchStart: track.branch.firstHour, active, onContextMenu: jump,
    axes: { xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: linesView && options.log === true } };
  if (view === 'race' && !ranked.length && (!remoteRows || remoteRows.total === 0) && presentation.raceEmptyState !== false)
    return <WidgetState kind="empty" message={empty} />;
  if (view === 'lines' && !lines.length && lineEmpty) return <WidgetState kind="empty" {...lineEmpty} />;
  const mixState = view === 'mix' && mixEmpty ? !mix ? <WidgetState kind="empty" icon={mixEmpty.icon} message={mixEmpty.message} />
    : !(linesView ? keyHours : mixHours).length ? <WidgetState kind="unavailable" message={mixEmpty.unavailable(mixName)} /> : null : null;
  const legend = (shown: readonly CountryRacePart[], map: ReadonlyMap<string, string>) => <Legend label={partsLabel} windowed={presentation.windowedLegend}
    items={shown.map(part => ({ id: part.id, label: part.label, color: part.color ?? map.get(part.id) ?? tokens.muted, value: part.value }))} />;
  const raceProps: import('./RaceChart').RaceChartProps = {
    label: raceLabelAt(chartLabels?.race, moment) ?? `${label}, ${dayLabel(moment)}`, items, live, slots, remoteRows,
    name: (key, item) => nameOf(raceTag(key, item), raceHour(item)), nameSamples: !entityLooks && presentation.countryLooks !== false ? COUNTRY_NAME_SAMPLES : undefined,
    flag: entity.flag ? (key, item) => entity.flag?.(raceTag(key, item)) : undefined,
    countryFlag: entity.countryFlag ? raceCountryFlag : undefined, formatTick, pickOwner: timePickOwner,
    onPick: onPick ? (key, item) => onPick(raceHour(item), raceTag(key, item)) : undefined,
    tooltip: (key, item) => {
      const tag = raceTag(key, item), hour = raceHour(item);
      showTip(hour, [tag]);
      return tipAt(tag, hour, entityLooks ? nameOf(tag, hour) : labelAt(hour)(tag).name);
    },
    onContextMenu: (key, x, y, item) => jump(raceHour(item), x, y, raceTag(key, item)), dateOverlay: dateOverlay ? dayLabel(moment) : null,
    empty, hoursPerSecond: common.hoursPerSecond, active, resetKey: resetKey ?? `${track.source.id}:${track.branch.id}`,
  };
  return <div className="country-race" data-detail={[...new Set(tags.flatMap(tag => [entity.nameFailure?.(tag)?.message, entity.countryFlag?.(tag)?.flagDetail]).filter(Boolean))].join("\n") || undefined}>
    {view === 'mix' && mix && <p className={`country-race-country${presentation.strongMixName ? ' is-strong' : ''}`}>{mixName}</p>}
    {view === 'race' && legend(raceLegendParts, colours)}
    {view === 'mix' && !linesView && presentation.mixLegend !== false && legend(mixParts, mixColours)}
    {view === 'lines' && !endLabels && (presentation.countryLooks !== false || entityLooks) && <Legend label={lineLegendLabel} windowed={presentation.windowedLegend}
      items={[...new Set(lines.map(line => line.tag))].map(tag => ({ id: tag, label: nameOf(tag), detail: entity.nameFailure?.(tag)?.message, color: country(tag) }))} />}
    {beforeChart}
    {mixState}
    <div className="country-race-chart" onPointerLeave={() => showTip(null)}>
      {view === 'race' ? <RaceChart {...raceProps} />
        : <TimeChart key={presentation.resetTimeView ? view : undefined} {...common} label={view === 'mix' && mix ? chartLabels?.mix?.(mixName) ?? `${label} over time`
          : chartLabels?.lines ?? `${label} over time`} layers={timeLayers} gaps={gaps} notRecorded={notRecorded} strip={strip} formatValue={formatAxis ?? formatTick}
          keyHours={timeHours} badges={badges?.length ? badges : undefined}
          onHover={hour => { if (hour === null) showTip(null); }}
          onViewChange={observeView}
          onSummaryView={onSummaryView} liveHead={liveHead} data={linesView ? lineChartData : undefined}
          onNavigateHour={navigation?.onNavigateHour} keyboardHour={navigation?.keyboardHour} keyboardRevision={navigation?.keyboardRevision}
          padding={linesView ? presentation.linePadding !== undefined
            ? { right: endLabels ? presentation.linePadding : CHART_LAYOUT.right } : endLabels ? { right: tokens.fontSize.body * 12 } : undefined : undefined}
          revision={revision} pickOwner={timePickOwner} onPick={onPick ? async (hour, context) => {
            if (!linesView) { onPick(hour, mixTagAt(hour) ?? undefined); return; }
            const owner = timePickOwner;
            const known = lineSummaries?.size && exactLineTags ? await exactLineTags(hour) : null;
            if (livePickOwner.current !== owner) return;
            let choices: string[];
            if (lineSummaries?.size && known) {
              const knownTags = new Set(known);
              const summaryLines = drawnLines.flatMap((line, index) => {
                const held = lineData?.read() ?? lineSummaries;
                const summary = held.get(line.id ?? line.tag) ?? held.get(line.tag);
                return summary && knownTags.has(line.tag) ? [{ key: String(index), summary, held: presentation.held !== false }] : [];
              });
              choices = [...new Set(pickSummaryLines(summaryLines, hour, new Set(summaryLines.map(line => line.key)), context)
                .map(key => drawnLines[Number(key)].tag))];
            } else choices = lineSummaries?.size ? []
              : pickLines(drawnLines.map(line => ({ key: line.tag, runs: line.runs, held: presentation.held !== false })), hour, context);
            if (choices.length) onPick(hour, choices.length === 1 ? choices[0] : undefined, choices);
          } : undefined}
          tooltip={(hour, _frame, hovered = []) => {
            if (hour < extent[0] || hour > extent[1]) { showTip(null); return null; }
            const alias = mixTagAt(hour);
            showTip(hour, timeTooltip ? NO_TAGS : linesView ? linesAt(hour, drawnLines) : alias ? [alias] : [], hovered);
            if (timeTooltip) {
              const forms = timeTooltip(hour) ?? (hovered.length ? [{ title: formatDateTime(hour), rows: [] }] : null);
              return forms ? fitForms(forms, hovered, presentation.tooltipKeep ?? 1) : null;
            }
            return view === 'lines' ? fitForms(lineTooltip?.(hour, linesAt(hour, lines), labelAt, country, colours)
              ?? [linesTooltip(hour, lines, labelAt, country, formatDateTime(hour), label, formatValue)], hovered)
              : alias ? tipAt(alias, hour, labelAt(hour)(alias).name, hovered) : null;
          }} overlay={timeOverlay} />}
    </div>
    {menu.menu}
  </div>;
}
