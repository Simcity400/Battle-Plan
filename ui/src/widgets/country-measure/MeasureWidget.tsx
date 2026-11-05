import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import { failureText } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Segmented } from '../../components/Segmented';
import CountryPicker from '../../components/CountryPicker';
import { CountryFlagUrl } from '../../components/CountryFlag';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import type { EventBadge } from '../../charts/badges';
import { dayLabel, drawEventTicks, drawLine, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import type { LegendItem } from '../../charts/Legend';
import { SECONDARY_DASH } from '../../charts/palette';
import { extentOf, lineEndHour, lineLabelHours, runAt } from '../../charts/series';
import type { Run } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, ChartPick } from '../../charts/TimeChart';
import { CHART_LAYOUT } from '../../charts/tokens';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { chooseDrillCountry, drillCountryCoverage, drillCountrySpans, readDrillCountry } from './drillCountry';
import { powerCountryLedger } from './powerCountryData';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { countryLineColours, uncolouredWarning } from '../../dashboard/countryLineColours';
import { useCountryLineStyles } from '../../dashboard/useCountryLineStyles';
import type { WidgetProps } from '../../dashboard/definitions';
import { clipRuns, isMember, MEMBERSHIP_PREPARING, membershipChanges, resolveCountries } from '../../dashboard/groups';
import { pinTime, resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { powerDecimals } from '../../dashboard/powerDecimals';
import { usePlaybackHour } from '../../dashboard/playback';
import { useFollowedProducts, useProduct } from '../../dashboard/products';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { powerBalanceIndex, powerBalanceKeyHours, powerBalanceRows, powerCoverageAt, powerSpendTicks, validatePowerBalance } from '../../dashboard/powerBalance';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { readHourAt, validateStabilityReply } from '../../dashboard/stabilityReply';
import type { OptionField } from '../../dashboard/options';
import type { CountryStabilityReply, FixedRun, Measure, MeasureReply, PoliticsLedgerReply } from '../../generated/dashboard';
import { formatBalance, nearestDrop, netSpend, rawAt, toRuns, validateMeasureReply } from './measureData';
import { MEASURE_OPTIONS, POWER_BREAKDOWNS } from './options';
import PowerDrill from './PowerDrill';
import { powerCountrySegments } from './powerDrillData';
import { choosePowerActor, pickPowerActor, powerActorDrillAction } from './powerActorPick';
import { powerMaximaModel } from './powerPickModel';
import type { PowerActorPick } from './powerActorPick';
import type { PowerBreakdown } from './powerDrillData';
import { drawPreparedCoverage } from '../../charts/draw';
import { timelinePrepared } from '../../dashboard/timelineGaps';
import { drawLineSummary, extentOfSummary, pickSummaryLines } from '../../charts/chartSummary';
import { clusterHour, markSummaryMatches } from '../../charts/markSummary';
import { usePreparedMeasure } from './preparedMeasure';
import PreparedDropList from './PreparedDropList';
import type { MarkCluster, MarkSummary } from '../../charts/markSummary';
import './measure.css';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];
const NO_FLAGS: readonly string[] = [];
/** Hovering within this many pixels of a spend tick snaps to it. */
const SNAP_PX = 6;
/** Beyond this many countries the tooltip lists balances only (no maximum or day details), so it fits the window. */
const CEILING_ROWS_MAX = 4;

/** Which balance a widget charts, and how it names it. */
export interface MeasureConfig {
  measure: Measure;
  /** The chart's name, such as "Political power". */
  title: string;
  /** The measure in running text, such as "political power". */
  noun: string;
  /** The unit a single country's end label names, such as "PP". */
  unit: string;
  /** The widget's settings, when it adds to the shared ones. */
  options?: readonly OptionField[];
  /** Draws the recorded maximum dashed (command power), named in running text. */
  ceiling?: string;
}

/** A drill title names its country once the dated name has loaded. */
const drillCountryName = (label: { name: string; known?: boolean }) => label.known === undefined ? 'Loading…' : label.name;

/** The balance line stays independent of captured income and attributed changes.
 * Missing capture retains the explicitly labelled net-drop fallback. */
export default function MeasureWidget({ widget, runtime, active, countrySlot, onDiagnostics, config }: WidgetProps & { config: MeasureConfig }) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(config.options ?? MEASURE_OPTIONS, widget.options);
  const showCeiling = config.ceiling != null && options.ceiling !== false;
  const digits = powerDecimals(options);
  const pinnedMode = options.show === 'income' || options.show === 'spending' ? options.show as PowerBreakdown : null;
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const [localPick, setLocalPick] = useState<{ owner: unknown; hour: number | null } | null>(null);
  const [localDrill, dispatchDrill] = useReducer(reduceDrill, CLOSED);
  const [drillMode, setDrillMode] = useState<PowerBreakdown>(pinnedMode ?? 'income');
  const [visibleFlags, setVisibleFlags] = useState<readonly string[]>(NO_FLAGS);
  const onVisibleItems = useCallback((items: readonly LegendItem[]) => {
    const tags = [...new Set(items.map(item => item.id))].sort();
    setVisibleFlags(before => before.length === tags.length && before.every((tag, index) => tag === tags[index]) ? before : tags);
  }, []);
  useEffect(() => { if (pinnedMode) setDrillMode(pinnedMode); }, [pinnedMode]);
  const drillAnchor = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const lastHour = track?.branch.lastHour ?? 0;
  const fallback = defaultCountryTag(track);
  const actorScope = useMemo(() => JSON.stringify(widget.countries), [widget.countries]);
  const directActor = widget.countries.kind === 'selected' ? fallback : null;
  const hasSavedCountry = Object.hasOwn(widget.options ?? {}, 'powerCountry');
  const captureScope = useMemo(() => JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id,
    config.measure, actorScope, directActor, hasSavedCountry, widget.options?.powerCountry, pinnedMode]), [track?.source.id, track?.source.revision, track?.branch.id, config.measure, actorScope, directActor, hasSavedCountry, widget.options?.powerCountry, pinnedMode]);
  const captureOwner = useMemo(() => ({ client: runtime.client, scope: captureScope }), [runtime.client, captureScope]);
  const [pickedActor, setPickedActor] = useState<{ owner: typeof captureOwner; pick: PowerActorPick } | null>(null);
  const actorPick = pickedActor?.owner === captureOwner ? pickedActor.pick : null;
  const setActorPick = useCallback((pick: PowerActorPick | null) => setPickedActor(pick ? { owner: captureOwner, pick } : null), [captureOwner]);
  // Only the newest chart pick still open may choose the breakdown's country and hour.
  const picks = useRef(0);
  const drill = actorPick ? localDrill : CLOSED;
  const localHour = localPick?.owner === captureOwner ? localPick.hour : undefined;
  const setLocalHour = (hour: number | null) => setLocalPick({ owner: captureOwner, hour });
  const clockHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || pinnedMode !== null && localHour === null || (drill.breakdown !== null && drill.follow)));
  const groups = useCountryGroups(runtime, track, active && (widget.countries.kind === 'group' || pinnedMode !== null || drill.breakdown !== null));
  const heldGroups = useRef<{ owner: typeof captureOwner; data?: NonNullable<typeof groups.data> }>({ owner: captureOwner });
  if (heldGroups.current.owner !== captureOwner) heldGroups.current = { owner: captureOwner };
  if (groups.data) heldGroups.current.data = groups.data;
  const groupData = groups.data ?? heldGroups.current.data;
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupData, lastHour, Number.MAX_SAFE_INTEGER), [widget.countries, fallback, groupData, lastHour]);
  const membershipKey = useMemo(() => JSON.stringify([resolved.tags, resolved.membership ? [...resolved.membership] : null]), [resolved.tags, resolved.membership]);
  const singleTag = resolved.tags.length === 1 ? resolved.tags[0] : null;
  const drillTag = actorPick ? actorPick.tag : singleTag;
  const choosingActor = actorPick !== null && actorPick.tag === null;
  useEffect(() => {
    dispatchDrill({ type: 'close' });
    picks.current++;
    setActorPick(null);
    setLocalPick(null);
  }, [captureOwner, setActorPick]);
  const savedCountry = useMemo(() => readDrillCountry(widget.options?.powerCountry, track, widget.countries),
    [widget.options?.powerCountry, track, widget.countries]);
  const origin = useRef<{ owner: typeof captureOwner; tag: string; hour: number } | null>(null);
  if (origin.current?.owner !== captureOwner) origin.current = null;
  if (pinnedMode && singleTag && track && window && !origin.current) origin.current = { owner: captureOwner, tag: singleTag, hour: window.lastHour };
  const selectedCountry = useMemo(() => savedCountry ?? (hasSavedCountry ? null
    : groupData && track && drillTag ? chooseDrillCountry(groupData.countryIdentities, track, drillTag,
      actorPick?.hour ?? origin.current?.hour ?? window?.lastHour ?? lastHour) : null),
    [savedCountry, hasSavedCountry, groupData, track, drillTag, actorPick?.hour, captureOwner]);
  const actorSpans = useMemo(() => selectedCountry && groupData && track ? drillCountrySpans(groupData.countryIdentities, selectedCountry, track, resolved.membership) : null,
    [selectedCountry, groupData, track, resolved.membership]);
  const drillTags = useMemo(() => actorSpans ? [...actorSpans.keys()] : [], [actorSpans]);
  const requestedTags = useMemo(() => [...new Set([...resolved.tags, ...drillTags])], [resolved.tags, drillTags]);
  const request = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-measure',
    params: { branchId: track.branch.id, measure: config.measure, tags: resolved.tags } } : null;
  const prepared = usePreparedMeasure(runtime.client, !pinnedMode && widget.countries.kind !== 'group' ? track?.source : undefined,
    track?.branch.id, config.measure, resolved.tags, window?.lastHour ?? lastHour, hoverHour, active, options.spendTicks === true);
  const reference = useProduct<MeasureReply>(runtime.client, request, active && !prepared.enabled, validateMeasureReply);
  const { data, error, retry } = prepared.enabled ? prepared : reference;
  const [hoverCluster, setHoverCluster] = useState<{ owner: unknown; cluster: MarkCluster | null } | null>(null);
  // Each opening is its own list (`id`), so its country and page start afresh.
  const [changeList, setChangeList] = useState<{ owner: unknown; id: number; tag: string; first: string; end?: string } | null>(null);
  const changeLists = useRef(0);
  const onMarkHover = useCallback((cluster: MarkCluster | null) => setHoverCluster(previous => previous?.owner === prepared.owner && previous.cluster === cluster
    ? previous : { owner: prepared.owner, cluster }), [prepared.owner]);
  // The exact accounting/maxima oracle is reached on hover or opening a drill.
  // Balance preparation never starts another whole history request on mount.
  const reachedDetails = !prepared.enabled || hoverHour !== null || drill.breakdown !== null || pinnedMode !== null;
  const ledgerAt = Math.floor(hoverHour ?? (pinnedMode ? localHour === null || localHour === undefined && widget.time.mode === 'follow'
    ? clockHour + (track?.axisOffset ?? 0) : localHour ?? window?.lastHour ?? lastHour
    : drillHour(drill, track ? Math.floor(clockHour) + track.axisOffset : null) ?? window?.lastHour ?? lastHour));
  const ledgerRequests = track ? requestedTags.map(tag => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'politics-ledger', params: { branchId: track.branch.id, tag, view: 'balance',
      from: Math.floor(ledgerAt / 24) * 24, hour: ledgerAt } })) : [];
  const ledger = useFollowedProducts<PoliticsLedgerReply>(runtime.client, ledgerRequests, ['from', 'hour'], active && reachedDetails, validatePowerBalance);
  const currency = config.measure === 'politicalPower' ? 'Political' : 'Command';
  const details = useMemo(() => new Map((ledger.data ?? []).map(reply => [reply.tag, powerBalanceIndex(reply, currency)])), [ledger.data, currency]);
  // The day and hour the shown ledgers answer; a ledger kept while the next day loads answers only its own.
  const ledgerAnswer = ledger.requests?.[0]?.params as { from: number; hour: number } | undefined;
  const detailsAnswer = (hour: number) => !ledgerAnswer
    || Math.floor(hour / 24) * 24 === ledgerAnswer.from && Math.floor(hour) <= ledgerAnswer.hour;
  const heldDetails = useRef<{ owner: typeof captureOwner; indexes: typeof details }>({ owner: captureOwner, indexes: details });
  if (heldDetails.current.owner !== captureOwner || ledger.data) heldDetails.current = { owner: captureOwner, indexes: details };
  const numericDetails = heldDetails.current.indexes;
  // Content refreshes retain recorded geometry while its labels load again.
  // The maximum comes from the recorded inputs; it never holds up the balance lines.
  const ceilingRequest = showCeiling && track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-stability',
    params: { branchId: track.branch.id, tags: resolved.tags, hour: null } } : null;
  const ceilingPrepared = usePreparedMeasure(runtime.client, showCeiling ? track?.source : undefined, track?.branch.id, 'maxCommandPower', resolved.tags,
    window?.lastHour ?? lastHour, hoverHour, active && showCeiling, false, 'country-stability', undefined, resolved.membership ?? undefined);
  const ceilingLegacy = useProduct<CountryStabilityReply>(runtime.client, ceilingRequest, active && reachedDetails && !ceilingPrepared.enabled, validateStabilityReply);
  const ceilingData = useMemo<CountryStabilityReply | undefined>(() => ceilingPrepared.enabled ? ceilingPrepared.states && {
    branchId: track!.branch.id, decimals: 5, warnings: ceilingPrepared.states.warnings, series: resolved.tags.map(tag => ({ tag, stability: [], warSupport: [], breakdown: null, recorded: true,
      maxCommandPower: ceilingPrepared.data?.series.find(row => row.tag === tag)?.runs ?? [], carried: ceilingPrepared.states!.states.filter(state => state.tag === tag && state.readHour != null && state.readHour !== state.hour)
        .map(state => ({ firstHour: state.hour, lastHour: state.hour, readHour: state.readHour! })) })),
  } : ceilingLegacy.data, [ceilingPrepared.enabled, ceilingPrepared.states, ceilingPrepared.data, ceilingLegacy.data, track, resolved.tags]);
  const ceiling = { data: ceilingData, error: ceilingPrepared.enabled ? ceilingPrepared.error : ceilingLegacy.error, retry: ceilingPrepared.enabled ? ceilingPrepared.retry : ceilingLegacy.retry };
  const maximaReply = useMemo(() => ceiling.data ? powerMaximaModel(ceiling.data) : undefined, [ceiling.data]);
  const heldMaxima = useRef<{ owner: typeof captureOwner; model?: ReturnType<typeof powerMaximaModel> }>({ owner: captureOwner });
  if (heldMaxima.current.owner !== captureOwner) heldMaxima.current = { owner: captureOwner };
  if (maximaReply && maximaReply.key !== heldMaxima.current.model?.key) heldMaxima.current.model = maximaReply;
  const maxima = showCeiling ? heldMaxima.current.model : undefined;
  const maximaKey = maxima?.key;
  const maximaByTag = useMemo(() => new Map((maxima?.series ?? []).map(item => [item.tag, item])), [maxima]);
  const balanceOwner = prepared.enabled ? prepared.owner : data;
  const pickOwner = useMemo(() => ({ owner: captureOwner, membership: membershipKey, balances: balanceOwner, maxima: maximaKey, showCeiling }),
    [captureOwner, membershipKey, balanceOwner, maximaKey, showCeiling]);

  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  // Names and colours as they were at the moment shown: the playhead, the end of a
  // date range, or the branch end.
  const styleAt = playhead ?? (widget.time.mode === 'range' && window ? window.lastHour : lastHour);
  const detailsThrough = shownUntil(playhead, (options.playback as PlaybackMode) ?? 'draw-in', options.afterPlayhead === true) ?? extent[1];
  const liveHour = track ? Math.floor(clockHour) + track.axisOffset : null;
  const pinnedFollowing = localHour === null || localHour === undefined && following;
  const pinnedAt = localHour === undefined ? playhead ?? extent[1] : localHour === null ? liveHour : localHour;
  const drillAt = pinnedMode ? pinnedAt : drillHour(drill, liveHour);
  const drillPlayhead = pinnedMode ? pinnedAt : drill.breakdown !== null && drill.follow ? liveHour : playhead;
  const drillThrough = shownUntil(drillPlayhead, (options.playback as PlaybackMode) ?? 'draw-in', options.afterPlayhead === true) ?? extent[1];
  const askedHover = useRef<number | null>(null);
  const [viewX, setViewX] = useState<readonly [number, number] | null>(null);
  // End labels floor the drawn range, so easing re-renders only when it reaches another hour.
  const onViewChange = useCallback((x: readonly [number, number]) => {
    const bounds: [number, number] = [Math.floor(x[0]), Math.floor(x[1])];
    setViewX(before => before?.[0] === bounds[0] && before[1] === bounds[1] ? before : bounds);
  }, []);
  const styles = useCountryLineStyles(runtime.client, track, resolved.tags, styleAt, active);
  const known = styles.known;
  const lookSegments = useMemo(() => {
    if (prepared.enabled) {
      const runs = new Map((data?.series ?? []).map(item => [item.tag, item.runs]));
      return (tag: string, hour: number) => {
        const run = runAt(runs.get(tag) ?? [], hour) as (FixedRun & { segmentFirstHour?: number }) | null;
        return run?.segmentFirstHour ?? run?.firstHour ?? null;
      };
    }
    const index = new Map((data?.series ?? []).map(item => [item.tag, powerCountrySegments(item.runs)]));
    return (tag: string, hour: number) => runAt(index.get(tag) ?? [], hour)?.firstHour ?? null;
  }, [data, prepared.enabled]);
  const flagTags = active && !pinnedMode && !(drill.breakdown && options.drill !== 'popout') ? visibleFlags : NO_FLAGS;
  const legendLooks = useCountryLooks(runtime.client, track, flagTags, styleAt, active, lookSegments, undefined, true, flagTags);

  useEffect(() => {
    const uncoloured = uncolouredWarning((data?.series ?? []).map(item => item.tag), known, !!styles.data);
    const warnings = [...(data?.warnings ?? []), ...(styles.data?.warnings ?? []),
      ...(styles.error ? [`Map colours could not be loaded: ${styles.error.message}`] : []), ...(groups.data?.warnings ?? []),
      ...(widget.countries.kind === 'group' && groups.error ? [`Country membership labels could not be loaded: ${groups.error.message}`] : []),
      ...(ceilingRequest && ceiling.error ? [`The ${config.ceiling} could not be loaded: ${ceiling.error.message}`] : []),
      ...(ceilingRequest ? ceiling.data?.warnings ?? [] : []),
      ...(ledger.data ?? []).flatMap(reply => reply.warnings),
      ...(ledger.error ? [`Income and causes could not be loaded: ${ledger.error.message}`] : []),
      ...(uncoloured ? [uncoloured] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, styles.data, styles.error, known, groups.data, groups.error, widget.countries.kind, onDiagnostics, ceilingRequest != null, ceiling.error, ceiling.data, config.ceiling, ledger.data, ledger.error]);

  const series = useMemo(() => {
    if (!data) return [];
    const colours = countryLineColours(data.series.map(item => item.tag), known, tokens.surface, tokens.countryColours === 'fixed');
    return data.series.map(item => {
      const membership = resolved.membership?.get(item.tag);
      const runs = toRuns(item, data.decimals);
      // A country counted at each hour shows nothing (line, value, spend) outside its membership.
      const drops = item.drops.filter(drop => isMember(membership, drop.hour));
      const ticks = powerSpendTicks(numericDetails.get(item.tag), drops, detailsThrough).filter(tick => isMember(membership, tick.hour));
      const valueAt = (hour: number) => isMember(membership, hour) ? rawAt(item, hour) : null;
      // The recorded maximum, clipped to the same membership as the balance.
      const maximum = maximaByTag.get(item.tag);
      const ceilingRuns = maximum && maxima ? toRuns({ runs: maximum.maxCommandPower }, maxima.decimals) : [];
      const ceilingAt = (hour: number) => maximum && isMember(membership, hour) ? rawAt({ runs: maximum.maxCommandPower }, hour) : null;
      // The read a shown maximum comes from: an older one is carried, so its time is shown.
      const ceilingReadAt = (hour: number) => maximum ? readHourAt(maximum.carried, hour) : null;
      return { tag: item.tag,
        hex: colours.get(item.tag)!, drops, ticks, valueAt, runs: membership ? clipRuns(runs, membership) : runs, membership,
        ceilingRuns: membership ? clipRuns(ceilingRuns, membership) : ceilingRuns, ceilingAt, ceilingReadAt };
    });
  }, [data, known, resolved.membership, tokens, maxima, maximaByTag, numericDetails, detailsThrough]);

  const endHours = useMemo(() => lineLabelHours(series, viewX?.[0] ?? extent[0], viewX?.[1] ?? extent[1],
    shownUntil(playhead, (options.playback as PlaybackMode) ?? 'draw-in', options.afterPlayhead === true)),
  [series, viewX, extent[0], extent[1], playhead, options.playback, options.afterPlayhead]);
  const wantedLabels = useMemo(() => [...series.flatMap(item => [
    { tag: item.tag, hour: Math.floor(styleAt) },
    ...(hoverHour === null ? [] : [{ tag: item.tag, hour: hoverHour }]),
    ...(endHours.has(item.tag) ? [{ tag: item.tag, hour: endHours.get(item.tag)! }] : []),
    ...(item.membership ? membershipChanges(item.membership, extent[0], extent[1], resolved.unobserved).map(change => ({ tag: item.tag, hour: change.hour })) : []),
  ])],
  [series, styleAt, hoverHour, endHours, extent[0], extent[1], resolved.unobserved]);
  // End labels and membership markers need names; artwork is read only for
  // mounted legend entries and the exact hovered country rows.
  // Every membership marker date stays admitted, so a long history never leaves later names unread.
  const nameHours = useMemo(() => Math.max(MAX_NAME_HOURS, new Set(wantedLabels.map(item => item.hour)).size), [wantedLabels]);
  const names = useCountryNamesAt(runtime.client, track, wantedLabels, active, nameHours);
  const hoverLabels = useCountryLabelsAt(runtime.client, track, hoverHour === null ? [] : series.map(item => ({ tag: item.tag, hour: hoverHour })), active);
  const labelAt = useCallback((hour: number) => (tag: string) => {
    const name = names.get(nameKey(hour, tag));
    return { ...hoverLabels.labelAt(hour)(tag), ...(name ? nameReadMetadata(name) : {}) };
  }, [names, hoverLabels.labelAt]);
  const labelVersion = useRef({ labelAt, revision: 0 });
  if (labelVersion.current.labelAt !== labelAt) labelVersion.current = { labelAt, revision: labelVersion.current.revision + 1 };
  const labels = { labelAt, revision: labelVersion.current.revision };
  const keyHours = useMemo(() => [...new Set(series.flatMap(item => [
    ...item.runs.map(run => run.firstHour), ...item.ticks.map(tick => tick.hour),
    ...(numericDetails.has(item.tag) ? powerBalanceKeyHours(numericDetails.get(item.tag)!) : []),
  ]))].sort((a, b) => a - b), [series, numericDetails]);

  // Joins and leaves of a group counted at each hour, as badges in the country's colour.
  const badges = useMemo<EventBadge[]>(() => series.flatMap(item => item.membership
    ? membershipChanges(item.membership, extent[0], extent[1], resolved.unobserved).map(change => ({
      hour: change.hour, glyph: change.change === 'joined' ? '+' : '−', label: `${labels.labelAt(change.hour)(item.tag).name} ${change.change}`, color: item.hex })) : []),
  [series, extent[0], extent[1], resolved.unobserved, labels.labelAt]);
  const menu = useContextMenu('Chart actions');
  const head = playhead ?? extent[1];
  const legendItems = useMemo(() => series.map(item => {
    const at = Math.floor(head);
    const raw = item.valueAt(at);
    return { id: item.tag, label: labels.labelAt(at)(item.tag).name, color: item.hex,
      value: raw == null || !data ? 'Not recorded' : formatBalance(raw, data.decimals, digits),
      flag: <CountryFlagUrl className="country-flag" url={legendLooks.flag(item.tag, at)}
        pending={legendLooks.flagPending(item.tag, at)} detail={legendLooks.flagDetail?.(item.tag, at)} reason={legendLooks.flagReason(item.tag, at)} /> };
  }), [series, head, labels.labelAt, data, legendLooks, digits]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  // The last hour drawn: the playhead, unless whole-campaign playback shows past it.
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const markSummaries = useMemo<readonly MarkSummary[]>(() => prepared.enabled && options.spendTicks ? series.flatMap(item => {
    const marks = prepared.summaryByTag.get(item.tag)?.marks;
    return marks ? [{ ...marks, placement: 'ticks' as const, color: series.length === 1 ? tokens.muted : item.hex,
      clusters: marks.clusters.map(cluster => ({ ...cluster, tag: item.tag })) }] : [];
  }) : [], [prepared.enabled, prepared.summaryByTag, options.spendTicks, series, tokens.muted]);
  const selectedLedger = useMemo(() => actorSpans && drillTag && selectedCountry ? powerCountryLedger(ledger.data ?? [], actorSpans, drillTag, selectedCountry.countryId) : undefined,
    [ledger.data, actorSpans, drillTag, selectedCountry]);
  const numericLedger = useMemo(() => actorSpans && drillTag && selectedCountry ? powerCountryLedger([...numericDetails.values()].map(index => index.reply), actorSpans, drillTag, selectedCountry.countryId) : undefined,
    [numericDetails, actorSpans, drillTag, selectedCountry]);
  const drillDetail = useMemo(() => numericLedger ? powerBalanceIndex(numericLedger, currency) : undefined, [numericLedger, currency]);
  const countrySegments = useMemo(() => actorSpans ? drillCountryCoverage(actorSpans) : [], [actorSpans]);
  const shownTag = actorSpans && drillAt !== null ? countryDrillTagAt(actorSpans, Math.floor(drillAt)) : null;
  const countryLabels = useCountryLabelsAt(runtime.client, track, (drill.breakdown !== null || pinnedMode && savedCountry) && shownTag && drillAt !== null
    ? [{ tag: shownTag, hour: Math.floor(drillAt) }] : [], active);
  const actorReason = hasSavedCountry && !savedCountry ? 'Saved country selection unavailable. Choose it again.'
    : !groupData ? 'Loading country…'
      : !selectedCountry || !actorSpans ? 'Country unavailable at this time.'
        : !shownTag ? 'Country unavailable at this time.' : null;
  const countryHeader = pinnedMode && countrySlot ? createPortal(<DatedCountryChip hour={drillAt === null ? null : Math.floor(drillAt)}
    label={savedCountry && shownTag && drillAt !== null ? countryLabels.labelAt(Math.floor(drillAt))(shownTag) : null}
    reason={hasSavedCountry ? actorReason : 'Saved country unavailable.'} />, countrySlot) : null;
  const drillView = (mode: PowerBreakdown, at: number | null, pick?: (hour: number) => void) => error || (ledger.error && !drillDetail)
    ? <WidgetState kind="error" what={pinnedMode ? undefined : 'the breakdown'} failure={error ?? ledger.error!} onRetry={error ? retry : ledger.retry} />
    : choosingActor ? <p className="drill-empty">Choose a country from the drawn lines for this moment.</p>
      : (hasSavedCountry && !savedCountry) || (groupData && (!selectedCountry || !actorSpans)) ? <WidgetState kind="unavailable" message={actorReason!} />
        : groups.error && !groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
          : !drillDetail || !data ? <WidgetState kind="loading" label="Loading income and causes..." />
          : <PowerDrill client={runtime.client} source={track?.source} branchId={track?.branch.id} bindings={actorSpans ?? undefined} countryId={selectedCountry?.countryId} index={drillDetail} digits={digits} pickOwner={pickOwner} presentationReady={selectedLedger !== undefined} mode={mode} hour={at} visibleThrough={Math.floor(drillThrough)} extent={extent}
            within={null} countries={countrySegments}
            playback={{ playhead: drillPlayhead, mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }}
            branchStart={track?.branch.firstHour ?? 0} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} tokens={tokens}
            onPick={pick} onContextMenu={(hour, x, y) => track && menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />;

  const layers = useMemo<ChartLayer[]>(() => {
    const single = series.length === 1;
    const list: ChartLayer[] = series.map(item => ({
      draw: (ctx, frame, chartTokens, state) => {
        if (prepared.enabled) {
          const summary = prepared.summaryByTag.get(item.tag)?.summary;
          if (summary) drawLineSummary(ctx, frame, chartTokens, summary, item.hex, false, state.playhead, { area: single && options.areaFill === true });
        } else drawLine(ctx, frame, chartTokens, item.runs, item.hex, { area: single && options.areaFill === true, until: state.playhead ?? undefined });
      },
      extent: (x: readonly [number, number]) => {
        if (!prepared.enabled) return extentOf(item.runs as Run[], x[0], Math.min(x[1], until ?? x[1]));
        const summary = prepared.summaryByTag.get(item.tag)?.summary;
        if (!summary || summary.to > Math.min(x[1], until ?? x[1]) || summary.to < x[0]) return null;
        return extentOfSummary(summary);
      },
    }));
    // Each country's maximum, dashed in its colour.
    for (const item of series) if (ceilingPrepared.enabled || item.ceilingRuns.length) list.push({
      draw: (ctx, frame, chartTokens, state) => {
        const summary = ceilingPrepared.summaryByTag.get(item.tag)?.summary;
        if (ceilingPrepared.enabled) { if (summary) drawLineSummary(ctx, frame, chartTokens, summary, item.hex, false, state.playhead, { dash: SECONDARY_DASH }); }
        else drawLine(ctx, frame, chartTokens, item.ceilingRuns, item.hex, { dash: SECONDARY_DASH, until: state.playhead ?? undefined });
      },
      extent: (x: readonly [number, number]) => {
        if (!ceilingPrepared.enabled) return extentOf(item.ceilingRuns as Run[], x[0], Math.min(x[1], until ?? x[1]));
        const summary = ceilingPrepared.summaryByTag.get(item.tag)?.summary;
        const buckets = summary && summary.to <= Math.min(x[1], until ?? x[1]) && summary.to >= x[0] ? summary.buckets.filter(bucket => bucket !== null) : [];
        return buckets.length ? [Math.min(...buckets.map(bucket => bucket.minValue)), Math.max(...buckets.map(bucket => bucket.maxValue))] : null;
      },
    });
    if (options.spendTicks && !prepared.enabled) list.push({ draw: (ctx, frame, chartTokens, state) => {
      for (const item of series) {
        if (prepared.enabled) {
          const marks = prepared.summaryByTag.get(item.tag)?.marks;
          if (marks && markSummaryMatches(marks, frame, state.playhead)) drawEventTicks(ctx, frame, chartTokens, marks.clusters.map(clusterHour), single ? chartTokens.muted : item.hex);
        } else drawEventTicks(ctx, frame, chartTokens, item.ticks.map(tick => tick.hour).filter(hour => state.playhead == null || hour <= state.playhead), single ? chartTokens.muted : item.hex);
      }
    } });
    if (prepared.enabled) list.push({ draw: (ctx, frame, chartTokens, state) => {
      const coverage = prepared.summaries?.coverage;
      if (coverage) for (const span of timelinePrepared(coverage, track?.branch.gaps ?? [], 0, frame.x[0], frame.x[1]))
        drawPreparedCoverage(ctx, frame, chartTokens, { ...span, complete: coverage.complete }, state.playhead);
    } });
    return list;
  }, [track?.branch.gaps, prepared.summaries, series, options.areaFill, options.spendTicks, until, prepared.enabled, prepared.summaryByTag, ceilingPrepared.enabled, ceilingPrepared.summaryByTag]);

  if (!track) return <WidgetState kind="empty" icon="sparkle" message={`Load a campaign to see ${config.noun}.`} />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (widget.countries.kind === 'group' && groups.error && !groupData) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  const groupNotice = widget.countries.kind === 'group' && groups.error && groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (resolved.reason) return <>{groupNotice}<WidgetState kind="empty" message={resolved.reason} /></>;
  // A group's members at hours still being prepared are not known yet: none is not proof of none.
  const preparing = <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  const branchKnown = resolved.known(track.branch.firstHour, track.branch.lastHour);
  if (pinnedMode) return <div className="country-measure">
    {countryHeader}
    {groups.error && groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {drillAt !== null && <p className="chart-line-key">{dayLabel(Math.floor(drillAt))} {!pinnedFollowing && <button type="button" className="button small"
      onClick={() => setLocalHour(null)}>Follow playback</button>}</p>}
    {singleTag ? drillView(pinnedMode, drillAt, hour => setLocalHour(Math.floor(hour))) : branchKnown ? <WidgetState kind="empty" message="Choose one country to see income and causes." /> : preparing}
    {ledger.error && drillDetail && <button type="button" onClick={ledger.retry}>Retry income and causes</button>}
    {menu.menu}
  </div>;
  if (error && !data) return <>{groupNotice}<WidgetState kind="error" failure={error} onRetry={retry} /></>;
  if (!data || (widget.countries.kind === 'group' && !groupData)) return <>{groupNotice}<WidgetState kind="loading" label={`Loading ${config.noun}…`} /></>;
  if (!prepared.enabled && series.every(item => !item.runs.length)) return <>{groupNotice}{branchKnown
    ? <WidgetState kind="unavailable" message={`No ${config.noun} was recorded for these countries.`} /> : preparing}</>;

  const decimals = data.decimals;
  const hoursPerPixel = (frameWidth: number, span: number) => span / Math.max(1, frameWidth);
  const main = <div className="country-measure-view">
    {prepared.enabled && prepared.preparing && !prepared.summaries?.series.some(line => line.summary.approximate?.length) && <p className="chart-line-key" role="status">Preparing the requested balance hours…</p>}
    {error && <WidgetState kind="error" failure={error} onRetry={retry} />}
    {prepared.summaries?.series.some(line => line.summary.approximate?.length) && <p className="chart-line-key"><i className="is-dashed" aria-hidden="true" />Approximate</p>}
    <Legend label="Countries" items={legendItems} windowed onVisibleItems={onVisibleItems} />
    {/* The dashed line is named, so it never rests on its dash pattern alone. */}
    {series.some(item => item.ceilingRuns.length) && <p className="chart-line-key"><i className="is-dashed" aria-hidden="true" />Dashed line: {config.ceiling}</p>}
    <div className="country-measure-chart">
      <TimeChart label={`${config.title} of ${series.map(item => labels.labelAt(Math.floor(head))(item.tag).name).join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        padding={{ right: options.endLabels ? CHART_LAYOUT.measureEndLabels : CHART_LAYOUT.right }} layers={layers} active={active} revision={`${head}:${data.branchId}:${series.length}:${viewport}:${labels.revision}:${digits}:${ledger.data ? 'details' : ledger.error?.message ?? 'loading'}`}
        onViewChange={onViewChange}
        keyHours={prepared.enabled ? undefined : keyHours} pickOwner={pickOwner}
        onSummaryView={prepared.enabled || ceilingPrepared.enabled ? view => { if (prepared.enabled) prepared.onSummaryView(view); if (ceilingPrepared.enabled) ceilingPrepared.onSummaryView(view); } : undefined}
        onNavigateHour={prepared.enabled ? prepared.onNavigateHour : undefined}
        keyboardHour={prepared.keyboardHour} keyboardRevision={prepared.keyboardRevision}
        onHover={prepared.enabled ? setHoverHour : undefined}
        markSummaries={prepared.enabled ? markSummaries : undefined}
        onMarkHover={prepared.enabled ? onMarkHover : undefined}
        markTooltip={cluster => {
          const label = cluster.tag ? labels.labelAt(cluster.firstHour)(cluster.tag) : null;
          const drop = cluster.tag && data.series.find(series => series.tag === cluster.tag)?.drops.find(drop => drop.hour === cluster.firstHour);
          return { title: cluster.firstHour === cluster.lastHour ? dayLabel(cluster.firstHour) : `${dayLabel(cluster.firstHour)}–${dayLabel(cluster.lastHour)}`,
            countryFlag: label ?? undefined, subtitle: label?.name,
            rows: [{ label: 'Recorded balance changes', value: cluster.count },
              ...(drop ? [{ label: 'Balance fell', value: netSpend(drop, data.decimals, digits) }] : [{ label: 'First balance change', value: 'Loading…' }])],
            note: 'Open Balance changes from the chart menu to reach each event.' };
        }}
        formatValue={value => Math.abs(value) >= 1000 ? `${Math.round(value / 100) / 10}k` : String(Math.round(value))}
        snap={(hour, frame) => {
          if (!options.spendTicks) return hour;
          const tolerance = SNAP_PX * hoursPerPixel(frame.plotWidth, frame.x[1] - frame.x[0]);
          // Only ticks that are drawn: inside the view and not after the last hour drawn.
          const last = Math.min(frame.x[1], until ?? frame.x[1]);
          const visible = (drop: { hour: number }) => drop.hour >= frame.x[0] && drop.hour <= last;
          const nearest = series.map(item => nearestDrop(item.ticks.filter(visible), hour, tolerance)).filter(drop => drop != null)
            .sort((a, b) => Math.abs(a!.hour - hour) - Math.abs(b!.hour - hour))[0];
          return nearest ? nearest.hour : hour;
        }}
        badges={badges.length ? badges : undefined}
        tooltip={(hour, _frame, hovered) => {
          if (!prepared.enabled && askedHover.current !== hour) {
            askedHover.current = hour;
            queueMicrotask(() => setHoverHour(hour));
          }
          const rows: TooltipRow[] = series.flatMap(item => {
            const raw = item.valueAt(hour);
            const label = labels.labelAt(hour)(item.tag);
            const balance: TooltipRow = { label: label.name, countryFlag: label, swatch: item.hex, value: raw == null ? prepared.enabled && !prepared.hourLoaded(item.tag, hour) ? 'Loading balance…' : 'Not recorded' : `${formatBalance(raw, decimals, digits)} ${config.unit}`, tone: raw == null ? 'muted' : undefined };
            if (!maxima || series.length > CEILING_ROWS_MAX) return [balance];
            const maximum = item.ceilingAt(hour);
            return [balance, { label: series.length > 1 ? `${label.name}: maximum` : 'Maximum', value: maximum == null ? 'Not recorded' : formatBalance(maximum, maxima.decimals, digits), tone: 'muted' }];
          });
          // Past a few countries the tooltip lists balances only, so it fits the window.
          for (const item of series.length > CEILING_ROWS_MAX ? [] : series) {
            if (!isMember(item.membership, hour) || item.valueAt(hour) === null) continue;
            const detail = detailsAnswer(hour) ? details.get(item.tag) : undefined;
            if (series.length > 1) rows.push({ label: labels.labelAt(hour)(item.tag).name, strong: true, divider: true });
            if (detail) rows.push(...powerBalanceRows(detail, hour, Math.floor(detailsThrough), digits));
            else rows.push({ label: 'Income and causes', detail: ledger.error?.message, value: ledger.error ? failureText(ledger.error, 'Couldn’t load income and causes.') : 'Loading…', tone: 'muted' });
            const drop = item.drops.find(entry => entry.hour === hour);
            const cover = detail && powerCoverageAt(detail, hour);
            const captured = currency === 'Political' ? cover?.political : cover?.command;
            if (drop && detail && !captured) rows.push({ label: 'Balance fell', value: netSpend(drop, decimals, digits), divider: true });
          }
          rows.push(...badgeRows(hovered));
          const notes = [
            ...(rows.some(row => row.label === 'Balance fell') ? ['Payments weren’t recorded here.'] : []),
          ];
          const content = { title: dayLabel(hour), rows, note: notes.length ? notes.join(' ') : undefined };
          return fitTooltip([content], viewport);
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          // At the end of the lines drawn, never past the recorded hours.
          const at = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          const ends = series.flatMap(item => {
            // Where the line ends, even when the playhead or view end lies in a gap after it.
            const last = lineEndHour(item.runs, frame.x[0], at);
            const raw = last == null ? null : item.valueAt(last);
            if (raw == null) return [];
            // One country: its value and unit; several: each named.
            const text = series.length === 1 ? { text: `${formatBalance(raw, decimals, digits)} ${config.unit}` }
              : { text: labels.labelAt(last!)(item.tag).name, suffix: formatBalance(raw, decimals, digits) };
            return [{ id: item.tag, y: frame.toY(Number(raw) / 10 ** decimals), ...text, color: item.hex }];
          });
          placeEndLabels(ctx, frame, chartTokens, ends);
        }}
        onPick={(hour, context?: ChartPick) => {
          const pick = ++picks.current;
          if (prepared.enabled) {
            void prepared.exactAt(hour).then(exact => {
              if (!exact || pick !== picks.current) return;
              const known = new Set(exact.states.filter(state => state.run !== null).map(state => state.tag));
              const candidates = pickSummaryLines([...prepared.summaryByTag].map(([key, value]) => ({ key, summary: value.summary })), hour, known, context);
              if (!candidates.length) return;
              const picked: PowerActorPick = { hour: Math.floor(hour), candidates, tag: candidates.length === 1 ? candidates[0] : null };
              setActorPick(picked);
              if (!drill.breakdown) setDrillMode(exact.states.some(state => state.tag === picked.tag && state.drop) ? 'spending' : 'income');
              dispatchDrill(powerActorDrillAction(picked, drill.breakdown !== null, runtime.clock.getSnapshot().playing, following ? playhead : null));
            });
            return;
          }
          const picked = pickPowerActor(series.flatMap(item => [
            { key: item.tag, runs: item.runs },
            ...(item.ceilingRuns.length ? [{ key: item.tag, runs: item.ceilingRuns }] : []),
          ]), hour, context);
          if (!picked) return;
          setActorPick(picked);
          if (!drill.breakdown) setDrillMode(picked.tag && series.find(item => item.tag === picked.tag)?.ticks.some(tick => tick.hour === picked.hour)
            ? 'spending' : 'income');
          dispatchDrill(powerActorDrillAction(picked, drill.breakdown !== null, runtime.clock.getSnapshot().playing, following ? playhead : null));
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour), ...(prepared.enabled && series.length ? [{ label: 'Balance changes', run: () => {
          const cluster = hoverCluster?.owner === prepared.owner ? hoverCluster.cluster : null;
          const tag = cluster?.tag ?? series[0].tag;
          const marks = prepared.summaryByTag.get(tag)?.marks;
          const first = cluster?.firstOrdinal ?? marks?.clusters[0]?.firstOrdinal ?? '0';
          const last = cluster ?? marks?.clusters.at(-1);
          // Without a drawn cluster (markers off, or none in view) the list is every recorded change.
          const end = last ? (BigInt(last.firstOrdinal) + BigInt(last.count)).toString() : undefined;
          setChangeList({ owner: prepared.owner, id: ++changeLists.current, tag, first, end });
        } }] : [])])} />
    </div>
    {prepared.enabled && changeList?.owner === prepared.owner && <PreparedDropList key={changeList.id} client={runtime.client} source={track.source} branchId={track.branch.id}
      measure={config.measure} tags={resolved.tags} initialTag={changeList.tag} firstOrdinal={changeList.first} endOrdinal={changeList.end}
      digits={digits} unit={config.unit} visibleThrough={until ?? extent[1]} label={tag => labels.labelAt(Math.floor(head))(tag).name}
      onClose={() => setChangeList(null)} onPick={(tag, hour) => {
        picks.current++;
        const picked: PowerActorPick = { hour, tag, candidates: [tag] }; setActorPick(picked); setDrillMode('spending');
        dispatchDrill(powerActorDrillAction(picked, drill.breakdown !== null, runtime.clock.getSnapshot().playing, following ? playhead : null));
      }} />}
    {ledger.error && <button type="button" onClick={ledger.retry}>Retry income and causes</button>}
  </div>;
  const breakdownOf = POWER_BREAKDOWNS.find(item => item.id === drillMode)!;
  return <div className="country-measure" ref={drillAnchor}>
    {/* Outside the drill, so an open breakdown never hides a later failure. */}
    {groupNotice}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null} anchor={drillAnchor}
      title={drill.breakdown !== null && shownTag && drillAt !== null ? `${drillCountryName(countryLabels.labelAt(Math.floor(drillAt))(shownTag))}: ${breakdownOf.label}` : breakdownOf.label}
      day={drillAt === null ? '' : dayLabel(Math.floor(drillAt))} following={drill.follow}
      canFollow={!choosingActor && drillTag !== null && liveHour !== null && liveHour >= track.branch.firstHour && liveHour <= track.branch.lastHour} returnFocus=".country-measure-chart canvas" onClose={() => {
        dispatchDrill({ type: 'close' });
        picks.current++;
        setActorPick(null);
      }} onFollow={() => { if (!choosingActor) dispatchDrill({ type: 'follow' }); }}
      onPin={selectedCountry && actorSpans ? () => {
        if (!selectedCountry) return;
        const actor = { ...widget, options: { ...widget.options, powerCountry: JSON.stringify(selectedCountry) }, countries: { kind: 'tags' as const, tags: [selectedCountry.tag] } };
        const source = drill.follow ? { ...actor, time: { mode: 'follow' as const } } : drillAt === null ? actor : { ...actor,
          time: pinTime({ ...actor, time: { mode: 'follow' } }, runtime.tracks, drillAt - track.axisOffset) };
        runtime.addWidgetBelow(widget.id, pinnedBreakdown(source, breakdownOf, track.id));
      } : undefined} main={main}>
      {choosingActor && actorPick && <CountryPicker countries={track.source.metadata.countries} playerCountryId={null}
        selectedTag={null} source={track.source} client={runtime.client} allowedTags={actorPick.candidates}
        flagAt={{ branchId: track.branch.id, hour: actorPick.hour }} onChange={tag => {
          const chosen = choosePowerActor(actorPick, tag);
          if (chosen === actorPick) return;
          picks.current++;
          setActorPick(chosen);
          dispatchDrill({ type: 'pick', hour: chosen.hour });
        }} />}
      <Segmented label="Breakdown of" value={drillMode} onChange={setDrillMode}
        options={POWER_BREAKDOWNS.map(item => ({ value: item.id as PowerBreakdown, label: item.label }))} />
      {drillView(drillMode, drillAt, hour => dispatchDrill({ type: 'pick', hour }))}
    </DrillHost>
    {menu.menu}
  </div>;
}
