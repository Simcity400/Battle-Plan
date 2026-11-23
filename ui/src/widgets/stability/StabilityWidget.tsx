import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { answersKey, countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { failureText } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { chooseDrillCountry, drillCountrySpans, readDrillCountry } from '../country-measure/drillCountry';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import StabilityDrill, { MEASURE_NAMES } from './StabilityDrill';
import CountryPicker from '../../components/CountryPicker';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import { dayLabel, END_LABEL_HEIGHT, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import type { LegendItem } from '../../charts/Legend';
import { SECONDARY_DASH } from '../../charts/palette';
import { runAt } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer, ChartPick } from '../../charts/TimeChart';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { CHART_LAYOUT } from '../../charts/tokens';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { drawLineSummary, pickSummaryLines, summaryExtent } from '../../charts/chartSummary';
import { motionOf } from '../../charts/easing.ts';
import LiveText, { liveTexts } from '../../charts/LiveText';
import { useCampaignSettings } from '../../dashboard/campaignSettings';
import { factionNameAt } from '../../dashboard/factionNames';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { countryLineColours, uncolouredWarning } from '../../dashboard/countryLineColours';
import { useCountryLineStyles } from '../../dashboard/useCountryLineStyles';
import { MAX_NAME_HOURS, isFailedFact } from '../../dashboard/countryFacts.ts';
import { validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import type { Failure } from '../../dashboard/queryError';
import { useFocusText } from '../national-focus/useFocusArtwork';
import { validateFocusReply } from '../national-focus/focusData';
import { validatePartyHistoryReply } from '../ruling-party/partyData';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { NumericReply } from '../../dashboard/numericReply';
import { groupNumbers } from '../../dashboard/groupsReply';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { nameReadFailure, nameReadMetadata, nameReadText } from '../../dashboard/nameReads.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import type { NameAt } from '../../dashboard/countryNamesAt';
import { followStore } from '../../dashboard/follow/store.ts';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { followRefusal, followsHead, usePlaybackHour, useFollow } from '../../dashboard/follow/useFollow.ts';
import { shownThrough } from '../../dashboard/follow/store.ts';
import type { FollowEntry } from '../../dashboard/follow/store';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { useIdentityHour } from '../../dashboard/atHead.ts';
import { trackHourOf } from '../../dashboard/settledHour.ts';
import { followAtHead } from '../../dashboard/tiles/head.ts';
import type { CountryGroupReply, FocusReply, IdeaHistoryReply, PartyHistoryReply, StabilityBreakdownReply, StabilityReadsReply } from '../../generated/dashboard';
import { useMeasureSeries } from '../country-measure/measureSeries';
import { STABILITY_BREAKDOWNS, STABILITY_OPTIONS } from './definition';
import { historyMarks, stabilityMarkers } from './markers';
import type { FocusMarks, MarkerKind } from './markers';
import { breakdownRows, formatPercent, linesPerMeasure, readHourOf, stabilityEnd, validateStabilityReads, zoomRange } from './stabilityData';
import type { Measure } from './stabilityData';
import { chooseStabilityActor, pinStabilityDrill, stabilityPickAction } from './stabilityDrillData';
import type { StabilityPick } from './stabilityDrillData';
import './stability.css';
import { useChangedState } from '../../dashboard/changedState.ts';
import { busyWait } from '../../shell/busy.ts';

const NO_WARNINGS: string[] = [];
const NO_FLAGS: readonly string[] = [];
const NO_PAINT = () => {};
const BREAKDOWN_RETRY_MS = 3000;
const BREAKDOWN_RETRIES = 3;
const FOCUS_SETTLE_MS = 400;
const END_LABEL_GAP = 2;
const MEASURES: readonly { key: Measure; label: string; dash?: readonly number[] }[] = [
  { key: 'stability', label: 'Stability' },
  { key: 'warSupport', label: 'War support', dash: SECONDARY_DASH },
];
const FAMILY = 'country-stability';

function validateBreakdown(value: unknown): StabilityBreakdownReply {
  const reply = value as StabilityBreakdownReply;
  if (typeof reply !== 'object' || reply === null || typeof reply.tag !== 'string' || !Number.isSafeInteger(reply.hour)
    || !(reply.breakdown === null || typeof reply.breakdown === 'object') || !(reply.missing === null || typeof reply.missing === 'string')
    || !Array.isArray(reply.warnings)) throw new Error('The stability breakdown reply is malformed.');
  return reply;
}

export default function StabilityWidget({ widget, runtime, active, countrySlot, onDiagnostics }: WidgetProps) {
  const tokens = useChartTokens();
  const campaignSettings = useCampaignSettings();
  const viewport = useViewportHeight();
  const options = resolveOptions(STABILITY_OPTIONS, widget.options);
  const pinnedMeasure = options.show === 'stability' || options.show === 'warSupport' ? options.show as Measure : null;
  const [localPick, setLocalPick] = useState<{ owner: unknown; hour: number | null } | null>(null);
  const [drill, dispatchDrill] = useReducer(reduceDrill, CLOSED);
  const [drillMeasure, setDrillMeasure] = useState<Measure>(pinnedMeasure ?? 'stability');
  const [visibleFlags, setVisibleFlags] = useChangedState<readonly string[]>(NO_FLAGS);
  const onVisibleItems = useCallback((items: readonly LegendItem[]) => {
    const tags = [...new Set(items.map(item => item.id))].sort();
    setVisibleFlags(before => before.length === tags.length && before.every((tag, index) => tag === tags[index]) ? before : tags);
  }, []);
  const [actorPick, setActorPick] = useState<{ owner: unknown; pick: StabilityPick } | null>(null);
  const [drillNotes, setDrillNotes] = useState<readonly string[]>([]);
  useEffect(() => { if (pinnedMeasure) setDrillMeasure(pinnedMeasure); }, [pinnedMeasure]);
  const shown = useMemo(() => MEASURES.filter(measure => options[measure.key] !== false), [options.stability, options.warSupport]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const hasSavedCountry = Object.hasOwn(widget.options ?? {}, 'stabilityCountry');
  const bindingKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries, hasSavedCountry, widget.options?.stabilityCountry, pinnedMeasure]);
  const bindingOwner = useMemo(() => ({ client: runtime.client, key: bindingKey }), [runtime.client, bindingKey]);
  const localHour = localPick?.owner === bindingOwner ? localPick.hour : undefined;
  const setLocalHour = (hour: number | null) => setLocalPick({ owner: bindingOwner, hour });
  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const [focused, setFocused] = useState(false);
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const groupStore = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const groupData = groupStore.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries]), groups.data, groupNumbers);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupData, lastHour, Number.MAX_SAFE_INTEGER), [widget.countries, fallback, groupData, lastHour]);
  const single = resolved.tags.length === 1 ? resolved.tags[0] : null;
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track.source, runtime.clock, track.axisOffset) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const headQuery = (measure: Measure) => following && active && track && resolved.tags.length && !pinnedMeasure ? { product: 'series-state', params: { branchId: track.branch.id,
    family: { kind: FAMILY, measure }, reads: resolved.tags.map(tag => ({ country: countryFromKey(tag), hour: 0 })) } } : null;
  const stabilityQuery = useMemo(() => headQuery('stability'), [following, active, track?.branch.id, resolved.tags, pinnedMeasure]);
  const warQuery = useMemo(() => headQuery('warSupport'), [following, active, track?.branch.id, resolved.tags, pinnedMeasure]);
  const reportShown = useShownHour();
  const stabilityFollowed = useFollow(store, stabilityQuery, root, useCallback((entry: FollowEntry) => reportShown(shownThrough(entry)), [reportShown]));
  const warFollowed = useFollow(store, warQuery, root, NO_PAINT);
  const headFollowed = followsHead(stabilityFollowed) && followsHead(warFollowed);
  const refused = followRefusal(stabilityFollowed, warFollowed);
  const clockHour = usePlaybackHour(runtime.clock, active && focused && single !== null && following);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = playhead ?? extent[1];
  const axisOffset = track?.axisOffset ?? 0;
  const clockHead = useMemo(() => ({ read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame }),
    [runtime.clock, axisOffset]);
  const liveHead = following ? clockHead : undefined;
  const dayText = useMemo(() => liveTexts(runtime.clock, () => Math.floor(clockHead.read()), hour => new Map([['day', dayLabel(hour)]])), [runtime.clock, clockHead]);
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const membership = resolved.membership ?? undefined;
  const stability = useMeasureSeries(runtime.client, track?.source, track?.branch.id, FAMILY, 'stability', resolved.tags,
    following ? null : head, hoverHour, active && !pinnedMeasure && options.stability !== false, false, membership, lastHour, following);
  const warSupport = useMeasureSeries(runtime.client, track?.source, track?.branch.id, FAMILY, 'warSupport', resolved.tags,
    following ? null : head, hoverHour, active && !pinnedMeasure && options.warSupport !== false, false, membership, lastHour, following);
  const series = { stability, warSupport } as const;
  const decimals = stability.decimals ?? warSupport.decimals ?? 0;
  const stabilityEntry = headFollowed ? stabilityFollowed.entry : null;
  const warEntry = headFollowed ? warFollowed.entry : null;
  const known = stability.decimals !== null || warSupport.decimals !== null;
  useEffect(() => {
    const heads = [[stabilityEntry, stability.head], [warEntry, warSupport.head]] as const;
    const stops = heads.flatMap(([entry, headOf]) => entry && known ? resolved.tags.map((tag, row) => followAtHead(headOf(tag), () => rowAmount(entry.answer, row, decimals))) : []);
    return () => stops.forEach(stop => stop());
  }, [stabilityEntry, warEntry, resolved.tags, stability.head, warSupport.head, decimals, known]);
  useEffect(() => { if (!following) reportShown(stability.states || warSupport.states ? head : null); }, [following, stability.states, warSupport.states, head, reportShown]);
  const chartData = useMemo<ChartData>(() => ({ revision: () => [stability.revision(), warSupport.revision()],
    subscribe: listener => { const stops = [stability.subscribe(listener), warSupport.subscribe(listener)]; return () => stops.forEach(stop => stop()); } }),
  [stability.revision, stability.subscribe, warSupport.revision, warSupport.subscribe]);

  const identityHour = useIdentityHour(runtime.client, track, resolved.tags, runtime.clock, active && following,
    hour => track ? trackHourOf(widget, track, runtime.tracks, hour) : null);
  const styleAt = Math.floor(following && identityHour !== null ? identityHour : playhead ?? (widget.time.mode === 'range' && window ? window.lastHour : lastHour));
  const styles = useCountryLineStyles(runtime.client, track, resolved.tags, styleAt, active);
  const lineByMeasure = useMemo(() => ({ stability: new Map(stability.lines.map(line => [line.tag, line])), warSupport: new Map(warSupport.lines.map(line => [line.tag, line])) }),
    [stability.lines, warSupport.lines]);
  const lookSegments = useCallback((tag: string, hour: number) => runAt(lineByMeasure.stability.get(tag)?.runs ?? [], hour)?.segmentFirstHour
    ?? runAt(lineByMeasure.warSupport.get(tag)?.runs ?? [], hour)?.segmentFirstHour ?? null, [lineByMeasure]);
  const flagTags = active && !(drill.breakdown && options.drill !== 'popout') ? visibleFlags : NO_FLAGS;
  const looks = useCountryLooks(runtime.client, track, flagTags, styleAt, active, lookSegments, undefined, true, flagTags);
  const lines = useMemo(() => {
    const colours = countryLineColours(resolved.tags, styles.known, tokens.surface, tokens.countryColours === 'fixed');
    return resolved.tags.map(tag => {
      const spans = membership?.get(tag);
      const valueAt = (measure: Measure, hour: number) => hour >= extent[0] && hour <= extent[1] && isMember(spans, hour)
        ? runAt(lineByMeasure[measure].get(tag)?.runs ?? [], hour)?.valueRaw ?? null : null;
      const missing = (measure: Measure, hour: number) => lineByMeasure[measure].get(tag)?.missing.get(Math.floor(hour));
      return { tag, hex: colours.get(tag)!, recordedColour: styles.known.has(tag), membership: spans, valueAt, missing };
    });
  }, [resolved.tags, styles.known, tokens.surface, tokens.countryColours, membership, lineByMeasure, extent[0], extent[1]]);
  const wantedLabels = useMemo(() => [...lines.map(item => ({ tag: item.tag, hour: styleAt })),
    ...(hoverHour === null ? [] : lines.map(item => ({ tag: item.tag, hour: hoverHour })))], [lines, styleAt, hoverHour]);
  const names = useCountryNamesAt(runtime.client, track, wantedLabels, active, MAX_NAME_HOURS);
  const hoverLabels = useCountryLabelsAt(runtime.client, track, hoverHour === null ? [] : lines.map(item => ({ tag: item.tag, hour: hoverHour })), active);
  const labelAt = useCallback((hour: number) => (tag: string) => {
    const name = names.get(nameKey(hour, tag));
    return { ...hoverLabels.labelAt(hour)(tag), ...(name ? nameReadMetadata(name) : {}) };
  }, [names, hoverLabels.labelAt]);
  const legendItems = useMemo(() => lines.map(item => ({ id: item.tag, label: labelAt(styleAt)(item.tag).name, color: item.hex,
    flag: <CountryFlagUrl className="country-flag" url={looks.flag(item.tag, styleAt)} pending={looks.flagPending(item.tag, styleAt)}
      detail={looks.flagDetail?.(item.tag, styleAt)} reason={looks.flagReason(item.tag, styleAt)} /> })), [lines, labelAt, looks, styleAt]);

  const kinds = useMemo(() => new Set(([['wars', 'markWars'], ['factions', 'markFactions'], ['government', 'markGovernment'], ['focus', 'markFocus']] as const)
    .filter(([, key]) => options[key] !== false).map(([kind]) => kind as MarkerKind)), [options.markWars, options.markFactions, options.markGovernment, options.markFocus]);
  const markerTag = single && kinds.size && !pinnedMeasure ? single : null;
  const singleSlots = useSlotTags(runtime.client, track, active && single !== null && isSlotKey(single));
  const markerName = markerTag && track ? keyNameAt(singleSlots, markerTag, track.branch.lastHour) : null;
  const factionOverrides = campaignSettings.campaigns[track?.source.metadata.campaignKey ?? '']?.factions;
  const markerRequest = (product: string, used: boolean) => track && markerTag && used ? { sourceId: track.source.id, revision: track.source.revision,
    product, params: { branchId: track.branch.id, country: countryFromKey(markerTag) } } : null;
  const markerIdeas = useProduct<IdeaHistoryReply>(runtime.client, markerRequest('idea-history', kinds.has('government')), active, validateIdeaHistoryReply, track?.source);
  const markerParties = useProduct<PartyHistoryReply>(runtime.client, markerRequest('party-history', kinds.has('government')), active, validatePartyHistoryReply, track?.source);
  const markerFocus = useProduct<FocusReply>(runtime.client, markerRequest('national-focus', kinds.has('focus')), active, validateFocusReply, track?.source);
  const focusMarks = useMemo<FocusMarks | undefined>(() => markerFocus.data && historyMarks(markerFocus.data), [markerFocus.data]);
  const markerSources = useMemo(() => !track || !markerTag ? null : {
    tag: markerName ?? markerTag, extent, branch: [track.branch.firstHour, track.branch.lastHour] as const,
    member: (hour: number) => isMember(resolved.membership?.get(markerTag), hour), groups: groupData,
    factionName: (faction: CountryGroupReply['factions'][number], hour: number) => factionNameAt(faction, hour, factionOverrides?.[faction.id]?.name),
    ideas: answersKey(markerIdeas.data?.asked, markerTag) ? markerIdeas.data : undefined, parties: answersKey(markerParties.data?.asked, markerTag) ? markerParties.data : undefined,
    focus: focusMarks,
  }, [track, markerTag, markerName, extent[0], extent[1], resolved.membership, groupData, factionOverrides, markerIdeas.data, markerParties.data, focusMarks]);
  const markerReads = useMemo(() => {
    const countries: NameAt[] = [], focuses: NameAt[] = [];
    if (markerSources) stabilityMarkers({ ...markerSources, countryName: (other, hour) => { countries.push({ hour, tag: other }); return ''; },
      focusName: (token, hour) => { focuses.push({ hour, tag: token }); return null; } }, kinds);
    return { countries, focuses, nameHours: Math.max(MAX_NAME_HOURS, new Set(countries.map(item => item.hour)).size) };
  }, [markerSources, kinds]);
  const namesAt = useCountryNamesAt(runtime.client, track, markerReads.countries, active, markerReads.nameHours);
  const focusText = useFocusText(runtime.client, track, markerTag ?? undefined, markerReads.focuses, active);
  const badges = useMemo(() => !markerSources ? [] : stabilityMarkers({ ...markerSources,
    countryName: (other, hour) => nameReadFailure(namesAt.get(nameKey(hour, other))) ?? nameReadText(namesAt.get(nameKey(hour, other))),
    focusName: (token, hour) => {
      const item = focusText.read.get(nameKey(hour, token));
      return isFailedFact(item) ? item : item === undefined ? undefined : item?.artwork.name ?? null;
    } }, kinds),
  [markerSources, namesAt, focusText.read, kinds]);

  const breakdownRequest = (hour: number | null) => track && single && hour !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-stability-breakdown', params: { branchId: track.branch.id, country: countryFromKey(single), hour: Math.floor(hour) } } : null;
  const hovered = useProduct<StabilityBreakdownReply>(runtime.client, breakdownRequest(!pinnedMeasure ? hoverHour : null), active, validateBreakdown, track?.source);
  const retries = useRef(0);
  useEffect(() => { retries.current = 0; }, [hoverHour, single]);
  const retryHovered = useRef(hovered.retry); retryHovered.current = hovered.retry;
  useEffect(() => {
    if (!hovered.error || hoverHour === null || !active || retries.current >= BREAKDOWN_RETRIES) return;
    return busyWait(() => { retries.current += 1; retryHovered.current(); }, BREAKDOWN_RETRY_MS * 2 ** retries.current);
  }, [hovered.error, hoverHour, active]);
  const readsRequest = track && lines.length > 1 ? { sourceId: track.source.id, revision: track.source.revision, product: 'stability-reads',
    params: { branchId: track.branch.id, from: Math.floor(extent[0]), to: Math.floor(extent[1]) } } : null;
  const inputReads = useProduct<StabilityReadsReply>(runtime.client, readsRequest, active, validateStabilityReads, track?.source);
  const [spokenAt, setSpokenAt] = useState<number | null>(null);
  useEffect(() => {
    if (!focused || !single || pinnedMeasure) return;
    return busyWait(() => setSpokenAt(Math.floor(head)), FOCUS_SETTLE_MS);
  }, [focused, single, head, pinnedMeasure]);
  useEffect(() => setSpokenAt(null), [track?.source.id, track?.source.revision, track?.branch.id, single]);
  const spoken = useProduct<StabilityBreakdownReply>(runtime.client, breakdownRequest(focused ? spokenAt : null), active, validateBreakdown, track?.source);

  const drillAnchor = useRef<HTMLDivElement | null>(null);
  const rootRef = useCallback((element: HTMLDivElement | null) => { drillAnchor.current = element; setRoot(element); }, []);
  const choiceScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries, widget.time, extent, resolved.tags,
    resolved.membership ? [...resolved.membership] : null, options.stability, options.warSupport, options.scale, options.show, options.playback, options.afterPlayhead, options.windowDays]);
  const pickOwner = useMemo(() => ({ scope: choiceScope, values: stability.owner }), [choiceScope, stability.owner]);
  const chosen = actorPick?.owner === pickOwner ? actorPick.pick : null;
  const drillOpen = drill.breakdown !== null && chosen !== null;
  const drillTag = chosen ? chosen.tag : single;
  const choosingActor = chosen !== null && chosen.tag === null;
  useEffect(() => { dispatchDrill({ type: 'close' }); setActorPick(null); }, [pickOwner]);
  const origin = useRef<{ owner: typeof bindingOwner; hour: number } | null>(null);
  if (origin.current?.owner !== bindingOwner) origin.current = null;
  if (pinnedMeasure && window && !origin.current) origin.current = { owner: bindingOwner, hour: window.lastHour };
  useEffect(() => setLocalPick(null), [bindingOwner]);
  const savedCountry = useMemo(() => readDrillCountry(widget.options?.stabilityCountry, track, widget.countries), [widget.options?.stabilityCountry, track, widget.countries]);
  const selectedCountry = useMemo(() => savedCountry ?? (hasSavedCountry ? null
    : groupData && track && drillTag ? chooseDrillCountry(groupData.countryIdentities, track, drillTag, chosen?.hour ?? origin.current?.hour ?? window?.lastHour ?? lastHour) : null),
  [savedCountry, hasSavedCountry, groupData, track, drillTag, chosen?.hour, bindingOwner]);
  const actorSpans = useMemo(() => selectedCountry && groupData && track ? drillCountrySpans(groupData.countryIdentities, selectedCountry, track, resolved.membership) : null,
    [selectedCountry, groupData, track, resolved.membership]);
  const liveHour = track ? Math.floor(clockHour) + track.axisOffset : null;
  const pinnedFollowing = localHour === null || localHour === undefined && following;
  const pinnedAt = localHour === undefined ? playhead ?? extent[1] : localHour === null ? liveHour : localHour;
  const drillAt = pinnedMeasure ? pinnedAt : drillOpen ? drillHour(drill, liveHour) : null;
  const drillFollows = pinnedMeasure ? pinnedFollowing && following : drillOpen && drill.follow;
  const drillPlayhead = pinnedMeasure ? pinnedAt : drillOpen && drill.follow ? liveHour : playhead;
  const shownTag = actorSpans && drillAt !== null ? countryDrillTagAt(actorSpans, Math.floor(drillAt)) : null;
  const drillWithin = shownTag && actorSpans ? actorSpans.get(shownTag) ?? null : null;
  const drillLabels = useCountryLabelsAt(runtime.client, track, (drillOpen || pinnedMeasure && savedCountry) && shownTag && drillAt !== null ? [{ tag: shownTag, hour: Math.floor(drillAt) }] : [], active);
  const actorReason = hasSavedCountry && !savedCountry ? 'Selected country unavailable.'
    : !groupData ? 'Loading country…' : !selectedCountry || !actorSpans || !shownTag ? 'Selected country unavailable.' : null;
  const countryHeader = pinnedMeasure && countrySlot ? createPortal(<DatedCountryChip hour={drillAt === null ? null : Math.floor(drillAt)}
    label={savedCountry && shownTag && drillAt !== null ? drillLabels.labelAt(Math.floor(drillAt))(shownTag) : null}
    reason={hasSavedCountry ? actorReason : 'Choose a country.'} />, countrySlot) : null;
  const markerNotes = useMemo(() => !markerTag ? [] : [
    ...((kinds.has('wars') || kinds.has('factions')) && groups.error ? [`Markers for wars and factions could not be loaded: ${groups.error.message}`] : []),
    ...(([['laws and advisors', markerIdeas, kinds.has('government')], ['ruling party', markerParties, kinds.has('government')],
      ['focuses', markerFocus, kinds.has('focus')]] as const).flatMap(([what, state, used]) => {
      if (!used) return [];
      const product = state as { error?: Failure; data?: { warnings: readonly string[] } };
      return product.error ? [`Markers for ${what} could not be loaded: ${product.error.message}`] : product.data?.warnings ?? [];
    })),
    ...(kinds.has('focus') ? focusText.warnings : []),
  ], [markerTag, kinds, groups.error, markerIdeas.data, markerIdeas.error, markerParties.data, markerParties.error, markerFocus.data, markerFocus.error, focusText.warnings]);
  useEffect(() => {
    const uncoloured = uncolouredWarning(resolved.tags, styles.known, !!styles.data);
    const warnings = [...(styles.data?.warnings ?? []), ...(styles.error ? [`Map colours could not be loaded: ${styles.error.message}`] : []),
      ...(groups.data?.warnings ?? []), ...(groups.error ? [`Country membership could not be loaded: ${groups.error.message}`] : []),
      ...(uncoloured ? [uncoloured] : []), ...(hovered.data?.warnings ?? []), ...(hovered.error ? [`The breakdown could not be loaded: ${hovered.error.message}`] : []),
      ...markerNotes, ...drillNotes];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [resolved.tags, styles.data, styles.error, styles.known, groups.data, groups.error, hovered.data, hovered.error, markerNotes, drillNotes, onDiagnostics]);
  const menu = useContextMenu('Chart actions');
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const layers = useMemo<ChartLayer[]>(() => lines.flatMap(item => shown.map(measure => ({
    draw: (ctx, frame, chartTokens, state) => {
      const summary = series[measure.key].lineOf(item.tag);
      if (summary) drawLineSummary(ctx, frame, chartTokens, summary, item.hex, false, state.playhead, { dash: measure.dash });
    },
    extent: (x: readonly [number, number]) => {
      const summary = series[measure.key].lineOf(item.tag);
      return summary ? summaryExtent(summary, x, until) : null;
    },
  }))), [lines, shown, until, stability.lineOf, warSupport.lineOf]);
  const error = stability.error ?? warSupport.error;
  const retry = () => { stability.retry(); warSupport.retry(); };

  if (!track) return <WidgetState kind="empty" icon="sliders" message="Load a campaign to see stability and war support." />;
  if (refused) return <WidgetState kind="unavailable" message={refused} />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (widget.countries.kind === 'group' && groups.error && !groupData) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  const groupNotice = widget.countries.kind === 'group' && groups.error && groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (resolved.reason) return <>{groupNotice}<WidgetState kind="empty" message={resolved.reason} /></>;
  const drill_ = (measure: Measure, onMeasure: ((measure: Measure) => void) | undefined, onPick: (hour: number) => void) =>
    <StabilityDrill client={runtime.client} clock={runtime.clock} source={track.source} branchId={track.branch.id} lastHour={lastHour}
      tag={shownTag} measure={measure} onMeasure={onMeasure} hour={drillAt} following={drillFollows} liveHead={drillFollows ? clockHead : undefined}
      playback={{ playhead: drillPlayhead, mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }}
      within={drillWithin} extent={extent} onPick={onPick} onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])}
      branchStart={track.branch.firstHour} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} tokens={tokens} onDiagnostics={setDrillNotes} />;
  if (pinnedMeasure) {
    const body = !single ? resolved.known(track.branch.firstHour, track.branch.lastHour) ? <WidgetState kind="empty" message="Choose one country to see what makes up its values." />
      : <WidgetState kind="loading" label="Loading what makes up the values…" />
      : (hasSavedCountry && !savedCountry) || (groupData && (!selectedCountry || !actorSpans)) ? <WidgetState kind="unavailable" message={actorReason!} />
        : groups.error && !groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
          : !groupData ? <WidgetState kind="loading" label="Loading what makes up the values…" />
            : drill_(pinnedMeasure, undefined, hour => setLocalHour(Math.floor(hour)));
    return <div className="stability is-pinned" ref={setRoot}>
      {countryHeader}
      {drillAt !== null && <p className="chart-line-key">{drillFollows ? <LiveText live={dayText} at="day" /> : formatDateTime(Math.floor(drillAt))} {!pinnedFollowing
        && <button type="button" className="button small" onClick={() => setLocalHour(null)}>Follow playback</button>}</p>}
      {body}
      {groups.error && groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
      {menu.menu}
    </div>;
  }
  if (error && !stability.summaries && !warSupport.summaries) return <>{groupNotice}<WidgetState kind="error" failure={error} onRetry={retry} /></>;
  if (widget.countries.kind === 'group' && !groupData) return <>{groupNotice}<WidgetState kind="loading" label="Loading stability and war support…" /></>;
  if (!shown.length) return <>{groupNotice}<WidgetState kind="empty" message="Both lines are switched off in the settings." /></>;

  const zoom = options.scale === 'zoom';
  const at = Math.floor(head);
  const answersSingle = (reply: StabilityBreakdownReply | undefined, hour: number | null) => !!reply && !!single && reply.hour === hour && reply.tag === keyNameAt(singleSlots, single, reply.hour);
  const spokenFor = answersSingle(spoken.data, spokenAt) ? spoken.data! : null;
  const spokenBreakdown = spokenFor?.breakdown ? shown.map(measure => breakdownRows(spokenFor.breakdown!, measure.key, decimals, measure.label, Number.MAX_SAFE_INTEGER).rows
    .map(row => row.value ? `${row.label} ${row.value}` : row.label).join(', ')).join('; ') : '';
  const spokenNote = !focused || spokenAt === null ? ''
    : spokenBreakdown ? `Breakdown on ${dayLabel(spokenAt)}${spokenFor!.breakdown!.readHour < spokenAt ? `, as of ${formatDateTime(spokenFor!.breakdown!.readHour)}` : ''}: ${spokenBreakdown}.`
      : spoken.error ? failureText(spoken.error, 'Couldn’t load the breakdown.')
        : spokenFor ? spokenFor.missing ?? `No breakdown was recorded for ${dayLabel(spokenAt)}.` : '';
  const hoveredFor = answersSingle(hovered.data, hoverHour) ? hovered.data! : null;
  const main = <div className="stability-main">
    <Legend label="Countries" items={legendItems} windowed onVisibleItems={onVisibleItems} />
    { }
    <p className="stability-key">{shown.map(measure => <span key={measure.key}><i aria-hidden="true" className={measure.dash ? 'is-dashed' : undefined} />{measure.dash ? 'Dashed' : 'Solid'}: {measure.label.toLowerCase()}</span>)}</p>
    { }
    <p className="sr-only" aria-live="off">{`${dayLabel(at)}: `}{lines.map(item => `${labelAt(styleAt)(item.tag).name}: ${shown.map(measure => {
      const raw = item.valueAt(measure.key, at);
      return `${measure.label} ${raw == null ? 'not recorded' : formatPercent(raw, decimals)}`;
    }).join(', ')}`).join('; ')}</p>
    <p className="sr-only" aria-live="polite" data-detail={spoken.error?.message}>{spokenNote}</p>
    <div className="stability-chart">
      <TimeChart label={`Stability and war support of ${lines.map(item => labelAt(styleAt)(item.tag).name).join(', ')}`} extent={extent} playhead={playhead} liveHead={liveHead} data={chartData}
        onSummaryView={view => { stability.onSummaryView(view); warSupport.onSummaryView(view); }}
        onNavigateHour={stability.onNavigateHour} keyboardHour={stability.keyboardHour} keyboardRevision={stability.keyboardRevision}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: zoom ? null : 0, yMax: zoom ? null : 1, log: false }}
        fitValues={zoom ? fitted => { const range = zoomRange(fitted && [fitted[0] * 100, fitted[1] * 100]); return range && [range[0] / 100, range[1] / 100]; } : undefined}
        padding={{ right: options.endLabels ? CHART_LAYOUT.countryEndLabels : CHART_LAYOUT.right }} layers={layers} active={active && !(drillOpen && options.drill !== 'popout')}
        notRecorded={stability.notRecorded && warSupport.notRecorded ? stability.notRecorded : null}
        pickOwner={pickOwner} revision={`${track.branch.id}:${lines.length}:${shown.length}:${viewport}:${hoveredFor ? 'breakdown' : hovered.error ? 'failed' : 'none'}:${stability.states ? 'states' : ''}`}
        formatValue={value => { const percent = value * 100; return `${Number.isInteger(Math.round(percent * 10) / 10) ? Math.round(percent) : percent.toFixed(1)}%`; }}
        onHover={hour => setHoverHour(hour === null ? null : Math.floor(hour))}
        badges={badges.length ? badges : undefined}
        tooltip={(hour, _frame, hoveredBadges) => {
          const rows: TooltipRow[] = [...badgeRows(hoveredBadges)];
          if (lines.length === 1) {
            const item = lines[0];
            const breakdown = hoveredFor?.breakdown ?? null;
            for (const [index, measure] of shown.entries()) {
              const raw = item.valueAt(measure.key, hour);
              const divider = index > 0 || rows.length > 0;
              if (raw == null) { rows.push({ label: measure.label, value: series[measure.key].hourLoaded(item.tag, hour) ? 'Not recorded' : 'Loading…', detail: item.missing(measure.key, hour), tone: 'muted', strong: true, divider }); continue; }
              if (!breakdown) { rows.push({ label: measure.label, value: formatPercent(raw, decimals), strong: true, divider }); continue; }
              const built = breakdownRows(breakdown, measure.key, decimals, measure.label, linesPerMeasure(shown.length));
              rows.push(...built.rows.map((row, position) => position === 0 ? { ...row, divider } : row));
            }
            const notes = [
              ...(hovered.error ? [failureText(hovered.error, 'Couldn’t load the breakdown.')]
                : hoveredFor && !hoveredFor.breakdown ? [hoveredFor.missing ?? 'No breakdown was recorded for this hour.']
                  : !hoveredFor ? ['Loading the breakdown…'] : []),
              ...(hoveredFor?.breakdown && hoveredFor.breakdown.readHour < hour ? [`As of ${formatDateTime(hoveredFor.breakdown.readHour)}.`] : []),
            ];
            return fitTooltip([{ title: dayLabel(hour), rows, detail: hovered.error?.message, note: notes.length ? notes.join(' ') : undefined }], viewport);
          }
          const read = inputReads.data ? readHourOf(inputReads.data.reads, Math.floor(hour)) : null;
          let carried = false;
          for (const item of lines) {
            const values = shown.map(measure => {
              const raw = item.valueAt(measure.key, hour);
              return `${measure.dash ? 'WS' : 'S'} ${raw == null ? '—' : formatPercent(raw, decimals)}`;
            });
            const none = shown.every(measure => item.valueAt(measure.key, hour) == null);
            const early = read !== null && read < Math.floor(hour) && !none;
            carried ||= early;
            rows.push({ label: labelAt(hour)(item.tag).name, swatch: item.hex, value: none ? 'Not recorded' : `${values.join(' · ')}${early ? ' †' : ''}`, tone: none ? 'muted' : undefined, short: true });
          }
          const note = [`${shown.map(measure => `${measure.dash ? 'WS' : 'S'}: ${measure.label.toLowerCase()}`).join(', ')}.`,
            ...(carried && read !== null ? [`† As of ${formatDateTime(read)}.`] : [])];
          return fitTooltip([{ title: dayLabel(hour), rows, note: note.join(' ') }], viewport);
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          const end = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          if (end < frame.x[0]) return;
          const ends = lines.flatMap(item => shown.flatMap(measure => {
            const at = stabilityEnd(series[measure.key].lineOf(item.tag), end, decimals);
            if (!at) return [];
            const text = `${lines.length > 1 ? `${labelAt(end)(item.tag).name}: ` : ''}${measure.label} ${formatPercent(at.raw, decimals)}`;
            return [{ id: `${item.tag}:${measure.key}`, y: frame.toY(at.value), text, color: item.hex }];
          }));
          const room = Math.floor((frame.plotHeight + END_LABEL_GAP) / (END_LABEL_HEIGHT + END_LABEL_GAP));
          const fitted = ends.length <= room ? ends : ends.filter(item => item.id.endsWith(':stability'));
          if (fitted.length > room) return;
          placeEndLabels(ctx, frame, chartTokens, fitted, END_LABEL_GAP);
        }}
        onPick={(hour, context?: ChartPick) => {
          const known = new Set(lines.flatMap(item => shown.flatMap(measure => item.valueAt(measure.key, Math.floor(hour)) != null ? [JSON.stringify([item.tag, measure.key])] : [])));
          const drawn = lines.flatMap(item => shown.flatMap(measure => {
            const summary = series[measure.key].lineOf(item.tag);
            return summary ? [{ key: JSON.stringify([item.tag, measure.key]), summary }] : [];
          }));
          const witnessed = pickSummaryLines(drawn, hour, known, context).map(key => { const [tag, measure] = JSON.parse(key) as [string, Measure]; return { tag, measure }; });
          const candidates = [...new Set(witnessed.map(line => line.tag))];
          if (!candidates.length) return;
          const picked: StabilityPick = { hour: Math.floor(hour), candidates, lines: witnessed, tag: candidates.length === 1 ? candidates[0] : null,
            measure: witnessed.find(line => line.measure === drillMeasure)?.measure ?? witnessed[0].measure };
          setActorPick({ owner: pickOwner, pick: picked });
          setDrillMeasure(picked.measure);
          dispatchDrill(stabilityPickAction(picked, drillOpen, widget.time.mode, runtime.clock.getSnapshot().playing, playhead));
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {error && <WidgetState kind="error" what="values" failure={error} onRetry={retry} />}
  </div>;
  const breakdownOf = STABILITY_BREAKDOWNS.find(item => item.id === drillMeasure) ?? STABILITY_BREAKDOWNS[0];
  return <div className="stability" ref={rootRef} onFocus={() => setFocused(true)}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false); }}>
    {groupNotice}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={drillAnchor}
      title={`${shownTag && drillAt !== null ? `${drillLabels.labelAt(Math.floor(drillAt))(shownTag).name}: ` : ''}What makes up ${MEASURE_NAMES[drillMeasure].toLowerCase()}`}
      day={drillAt == null ? '' : drillFollows ? <LiveText live={dayText} at="day" /> : dayLabel(Math.floor(drillAt))}
      following={drill.follow} canFollow={!choosingActor && drillTag !== null && liveHour !== null && liveHour >= track.branch.firstHour && liveHour <= track.branch.lastHour}
      returnFocus=".stability-chart canvas" onClose={() => { dispatchDrill({ type: 'close' }); setActorPick(null); }}
      onFollow={() => { if (!choosingActor && drillTag) dispatchDrill({ type: 'follow' }); }}
      onPin={selectedCountry && actorSpans && drillAt !== null ? () => runtime.addWidgetBelow(widget.id,
        pinStabilityDrill({ ...widget, options: { ...widget.options, stabilityCountry: JSON.stringify(selectedCountry) } }, breakdownOf, track.id, track.branch.firstHour, selectedCountry.tag, drillAt, drill.follow)) : undefined}
      main={main}>
      {choosingActor && chosen && <CountryPicker countries={track.source.metadata.countries} playerCountryId={null}
        selectedTag={null} source={track.source} client={runtime.client} allowedTags={chosen.candidates}
        flagAt={{ branchId: track.branch.id, hour: chosen.hour }} onChange={tag => {
          const next = chooseStabilityActor(chosen, tag);
          if (next === chosen) return;
          setActorPick({ owner: pickOwner, pick: next });
          setDrillMeasure(next.measure);
          dispatchDrill({ type: 'pick', hour: next.hour });
        }} />}
      {choosingActor ? <p className="drill-empty">Choose a country from the drawn lines for this moment.</p>
        : hasSavedCountry && !savedCountry ? <WidgetState kind="unavailable" message={actorReason!} />
        : groupData && (!selectedCountry || !actorSpans) ? <WidgetState kind="unavailable" message={actorReason!} />
        : groups.error && !groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
        : !groupData ? <WidgetState kind="loading" label="Loading what makes up the values…" />
        : drill_(drillMeasure, setDrillMeasure, hour => dispatchDrill({ type: 'pick', hour }))}
    </DrillHost>
    {menu.menu}
  </div>;
}
