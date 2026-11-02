import { type Failure, failureText } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { chooseDrillCountry, drillCountrySpans, readDrillCountry } from '../country-measure/drillCountry';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import type { StabilityPartsReply } from '../../generated/dashboard';
import StabilityDrill, { MEASURE_NAMES } from './StabilityDrill';
import { validateStabilityPartsReply } from './partsData';
import CountryPicker from '../../components/CountryPicker';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import { dayLabel, drawLine, END_LABEL_HEIGHT, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import type { LegendItem } from '../../charts/Legend';
import { nudgeApart, SECONDARY_DASH } from '../../charts/palette';
import { extentOf, lineEndHour } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, ChartPick } from '../../charts/TimeChart';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { CHART_LAYOUT } from '../../charts/tokens';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { useCampaignSettings } from '../../dashboard/campaignSettings';
import { factionNameAt } from '../../dashboard/factionNames';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { useFocusText } from '../national-focus/useFocusArtwork';
import { MAX_NAME_HOURS, isFailedFact } from '../../dashboard/countryFacts.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, MEMBERSHIP_PREPARING, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct, useProducts } from '../../dashboard/products';
import { readHourAt, validateStabilityReply } from '../../dashboard/stabilityReply';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { NumericReply } from '../../dashboard/numericReply';
import { groupNumbers } from '../../dashboard/groupsReply';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { nameReadText, nameReadFailure } from '../../dashboard/nameReads.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryGroupReply, CountryStabilityReply } from '../../generated/dashboard';
import { parseColor, toHex } from '../../shell/appearance/color';
import { STABILITY_BREAKDOWNS, STABILITY_OPTIONS } from './definition';
import { stabilityMarkers } from './markers';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import type { NameAt } from '../../dashboard/countryNamesAt';
import type { MarkerKind } from './markers';
import type { PartyHistoryReply } from '../../generated/dashboard';
import { usePreparedIdeaHistory } from '../../dashboard/ideaHistory';
import { validatePartyHistoryReply, partyBadges } from '../ruling-party/partyData';
import { contentTitle } from '../../dashboard/contentTitles';
import PartyEventList from '../ruling-party/PartyEventList';
import { partyMarkSummaries, usePreparedParty } from '../ruling-party/preparedParty';
import { usePartyParity } from '../ruling-party/partyParity';
import type { FocusPreparedClient } from '../national-focus/preparedFocusData';
import { useFocusMarks } from './focusMarks';
import { breakdownRows, formatPercent, linesPerMeasure, rawAt, zoomRange } from './stabilityData';
import type { Measure } from './stabilityData';
import { chooseStabilityActor, pickStabilityLine, pinStabilityDrill, stabilityNumbers, stabilityPartsNumbers, stabilityPickAction, stabilityPlotSeries, stabilityLookSegments, stabilityCountryParts } from './stabilityDrillData';
import type { StabilityPick } from './stabilityDrillData';
import { usePreparedScalarParity } from '../country-measure/usePreparedScalarParity';
import { percentSummary, usePreparedStability } from './preparedStability';
import { drawLineSummary, pickSummaryLines } from '../../charts/chartSummary';
import './stability.css';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];
const NO_FLAGS: readonly string[] = [];
/** A failed breakdown is asked for again after this delay, doubling each time, this many times. */
const BREAKDOWN_RETRY_MS = 3000;
const BREAKDOWN_RETRIES = 4;
/** With focus in the widget, the breakdown of the moment shown loads once playback rests this long. */
const FOCUS_SETTLE_MS = 400;
/** The gap kept between end labels. */
const END_LABEL_GAP = 2;
const MEASURES: readonly { key: Measure; label: string; dash?: readonly number[] }[] = [
  { key: 'stability', label: 'Stability' },
  { key: 'warSupport', label: 'War support', dash: SECONDARY_DASH },
];

/** Stability and war support over time: each country in its
 * map colour, stability solid and war support dashed, on a fixed 0–100% scale by
 * default. Hovering one country shows the game's own breakdown of both values for
 * the read behind that hour. */
export default function StabilityWidget({ widget, runtime, active, countrySlot, onDiagnostics }: WidgetProps) {
  const tokens = useChartTokens();
  const campaignSettings = useCampaignSettings();
  const viewport = useViewportHeight();
  const options = resolveOptions(STABILITY_OPTIONS, widget.options);
  const pinnedMeasure = options.show === 'stability' || options.show === 'warSupport' ? options.show as Measure : null;
  const [localPick, setLocalPick] = useState<{ owner: unknown; hour: number | null } | null>(null);
  const [drill, dispatchDrill] = useReducer(reduceDrill, CLOSED);
  const [drillMeasure, setDrillMeasure] = useState<Measure>(pinnedMeasure ?? 'stability');
  const [visibleFlags, setVisibleFlags] = useState<readonly string[]>(NO_FLAGS);
  const onVisibleItems = useCallback((items: readonly LegendItem[]) => {
    const tags = [...new Set(items.map(item => item.id))].sort();
    setVisibleFlags(before => before.length === tags.length && before.every((tag, index) => tag === tags[index]) ? before : tags);
  }, []);
  const [actorPick, setActorPick] = useState<{ owner: unknown; pick: StabilityPick } | null>(null);
  useEffect(() => { if (pinnedMeasure) setDrillMeasure(pinnedMeasure); }, [pinnedMeasure]);
  const shown = useMemo(() => MEASURES.filter(measure => options[measure.key] !== false), [options.stability, options.warSupport]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const hasSavedCountry = Object.hasOwn(widget.options ?? {}, 'stabilityCountry');
  const bindingKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries, hasSavedCountry, widget.options?.stabilityCountry, pinnedMeasure]);
  const bindingOwner = useMemo(() => ({ client: runtime.client, key: bindingKey }), [runtime.client, bindingKey]);
  const localHour = localPick?.owner === bindingOwner ? localPick.hour : undefined;
  const setLocalHour = (hour: number | null) => setLocalPick({ owner: bindingOwner, hour });
  const clockHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || pinnedMeasure !== null && localHour === null || (drill.breakdown !== null && drill.follow)));
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active && (widget.countries.kind === 'group' || pinnedMeasure !== null || drill.breakdown !== null));
  const fallback = defaultCountryTag(track);
  const groupStore = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const groupData = groupStore.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries]), groups.data, groupNumbers);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupData, lastHour, Number.MAX_SAFE_INTEGER), [widget.countries, fallback, groupData, lastHour]);
  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const [preparedHover, setPreparedHover] = useState<number | null>(null);
  const prepared = usePreparedStability(runtime.client, track?.source, track?.branch.id, resolved.tags, playhead ?? extent[1], preparedHover, active, shown.map(measure => measure.key), resolved.membership ?? undefined);
  const request = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-stability',
    params: { branchId: track.branch.id, tags: resolved.tags, hour: null } } : null;
  const product = useProduct<CountryStabilityReply>(runtime.client, request, active && !prepared.enabled, validateStabilityReply);
  const { error, retry } = prepared.enabled ? prepared : product;
  const numberStore = useMemo(() => new NumericReply<CountryStabilityReply>(), [runtime.client]);
  const numbers = numberStore.update(JSON.stringify(request), prepared.enabled ? prepared.data : product.data, stabilityNumbers);
  const data = (prepared.enabled ? prepared.data : product.data) ?? numbers;

  const styleAt = playhead ?? (widget.time.mode === 'range' && window ? window.lastHour : lastHour);
  const segmentAt = useMemo(() => stabilityLookSegments(numbers), [numbers]);
  const flagTags = active && !(drill.breakdown && options.drill !== 'popout') ? visibleFlags : NO_FLAGS;
  const looks = useCountryLooks(runtime.client, track, resolved.tags, styleAt, active, segmentAt, undefined, false, flagTags);

  // The breakdown of the one country shown, for the read behind the hovered hour,
  // without the histories (breakdownOnly), each read fetched once and cached.
  const single = resolved.tags.length === 1 ? data?.series.find(series => series.tag === resolved.tags[0]) : undefined;
  // Each read is asked for at one shown hour of it (the read's own hour may not be
  // readable), kept per country and read so revisiting reuses the cached reply;
  // an hour that turns out to be a gap is replaced by the next hovered hour.
  const [hover, setHover] = useState<{ tag: string; read: number; hour: number } | null>(null);
  const hoverKey = useRef<string | null>(null);
  const requestHours = useRef(new Map<string, number>());
  const emptyHours = useRef(new Set<string>());
  const retries = useRef(0);
  const tipHour = useRef<number | null>(null);
  const hoverRead = hover && single && hover.tag === single.tag ? hover.read : null;
  const breakdownRequest = track && single && hover && hover.tag === single.tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-stability',
    params: { branchId: track.branch.id, tags: [single.tag], hour: hover.hour, breakdownOnly: true } } : null;
  const breakdown = useProduct<CountryStabilityReply>(runtime.client, breakdownRequest, active, validateStabilityReply);
  // Only the hovered country's breakdown of the hovered read is shown.
  const replied = breakdown.data?.series[0];
  const breakdownFor = replied && single && replied.tag === single.tag && replied.breakdown?.readHour === hoverRead ? replied.breakdown : null;
  // A new recording, branch or country starts afresh.
  useEffect(() => {
    setHover(null); hoverKey.current = null; requestHours.current.clear(); emptyHours.current.clear();
  }, [track?.source.id, track?.source.revision, track?.branch.id, single?.tag]);
  useEffect(() => {
    // An hour with no breakdown, or whose lines were not recorded, is asked again at the next hour hovered.
    if (breakdown.data && hover && (!replied?.breakdown || !replied.breakdown.linesAvailable)) emptyHours.current.add(`${hover.tag}:${hover.read}:${hover.hour}`);
  }, [breakdown.data, hover, replied]);
  // A failed breakdown is asked for again after a growing delay, a few times, even
  // with the pointer still; hovering the read again starts the attempts afresh.
  useEffect(() => { retries.current = 0; }, [hover]);
  // The latest retry, as useProduct makes a new one each render.
  const retryBreakdown = useRef(breakdown.retry);
  retryBreakdown.current = breakdown.retry;
  useEffect(() => {
    if (!breakdown.error || !hover || !active || retries.current >= BREAKDOWN_RETRIES) return;
    const timer = setTimeout(() => {
      retries.current += 1;
      retryBreakdown.current();
    }, BREAKDOWN_RETRY_MS * 2 ** retries.current);
    return () => clearTimeout(timer);
  }, [breakdown.error, hover, active]);


  const geometry = useMemo(() => stabilityPlotSeries(numbers, resolved.membership, extent),
    [numbers, resolved.membership, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const colorKey = useMemo(() => JSON.stringify(geometry.map(item => looks.color(item.tag, styleAt))), [geometry, looks, styleAt]);
  // Colour adjustment is repeated only when the actual colour operands change.
  const colours = useMemo(() => {
    const raw = JSON.parse(colorKey) as ([number, number, number] | null)[];
    const recorded = raw.flatMap(color => color ? [{ r: color[0], g: color[1], b: color[2] }] : []);
    const adjusted = tokens.countryColours === 'fixed'
      ? nudgeApart(recorded, parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : recorded;
    let at = 0;
    return raw.map(color => ({ recordedColour: color !== null, hex: color ? toHex(adjusted[at++]) : tokens.muted }));
  }, [colorKey, tokens.countryColours, tokens.surface, tokens.muted]);
  const series = useMemo(() => geometry.map((item, index) => ({ ...item, ...colours[index], name: looks.name(item.tag, styleAt) })),
    [geometry, colours, looks, styleAt]);

  const legendItems = useMemo(() => series.map(item => ({ id: item.tag, label: item.name, color: item.hex,
    flag: <CountryFlagUrl className="country-flag" url={looks.flag(item.tag, styleAt)}
      pending={looks.flagPending(item.tag, styleAt)} detail={looks.flagDetail?.(item.tag, styleAt)} reason={looks.flagReason(item.tag, styleAt)} /> })),
    [series, looks, styleAt]);

  // Event markers for one country: each kind can be switched off.
  const kinds = useMemo(() => new Set(([['wars', 'markWars'], ['factions', 'markFactions'], ['government', 'markGovernment'], ['focus', 'markFocus']] as const)
    .filter(([, key]) => options[key] !== false).map(([kind]) => kind as MarkerKind)), [options.markWars, options.markFactions, options.markGovernment, options.markFocus]);
  // A widget showing only a breakdown draws no markers, so asks for none.
  const partsOnly = options.show === 'stability' || options.show === 'warSupport';
  const markerTag = resolved.tags.length === 1 && kinds.size && !partsOnly ? resolved.tags[0] : null;
  const markerGroups = useCountryGroups(runtime, track, active && markerTag != null && (kinds.has('wars') || kinds.has('factions')));
  const markerRequest = (product: string) => track && markerTag ? { sourceId: track.source.id, revision: track.source.revision, product, params: { branchId: track.branch.id, tag: markerTag } } : null;
  const [partyHover,setPartyHover] = useState<number | null>(null);
  const preparedParties = usePreparedParty(runtime.client,track?.source,track?.branch.id,markerTag ?? undefined,playhead ?? extent[1],partyHover,active && kinds.has('government'),['ruling'],false,markerTag ? resolved.membership?.get(markerTag) : undefined);
  const oldParties = useProduct<PartyHistoryReply>(runtime.client,kinds.has('government') ? markerRequest('party-history') : null,active && !preparedParties.enabled,validatePartyHistoryReply);
  const markerParties = preparedParties.enabled ? {...preparedParties,data:preparedParties.summary?.reply} : oldParties;
  const partyMember = useCallback((hour:number)=>isMember(markerTag ? resolved.membership?.get(markerTag) : undefined,hour),[markerTag,resolved.membership]);
  const partyParity = import.meta.env.MODE==='parity' ? usePartyParity(runtime.client,track?.source,track?.branch.id,kinds.has('government') ? markerTag ?? undefined : undefined,extent,'Stability party markers',oldParties.data,markerTag ? resolved.membership?.get(markerTag) : undefined) : undefined;
  const partyEvents = (reply:PartyHistoryReply | undefined) => reply ? partyBadges(reply,token=>contentTitle(reply.ideologies.find(item=>item.token===token)?.name,token,'Ideology name unavailable')).filter(badge=>badge.glyph!=='\u2713' && partyMember(badge.hour)) : [];
  const partyPageBadges = partyEvents(preparedParties.events?.reply).map(badge=>({...badge,glyph:'\u2699'}));
  const partyReachedBadges = partyEvents(preparedParties.reached?.reply).map(badge=>({...badge,glyph:'\u2699'}));
  const markerFocus = useFocusMarks(runtime.client as FocusPreparedClient, track, kinds.has('focus') ? markerTag ?? undefined : undefined, active);
  // The idea history of the marker country itself (a group may resolve to another country).
  const markerIdeas = usePreparedIdeaHistory(runtime.client, track, kinds.has('government') ? markerTag ?? undefined : undefined, track?.branch.lastHour, false, active);
  const factionOverrides = campaignSettings.campaigns[track?.source.metadata.campaignKey ?? '']?.factions;
  const markerSources = useMemo(() => !track || !markerTag ? null : {
    tag: markerTag, extent, branch: [track.branch.firstHour, track.branch.lastHour] as const,
    member: (hour: number) => isMember(resolved.membership?.get(markerTag), hour),
    groups: markerGroups.data, factionName: (faction: CountryGroupReply['factions'][number], hour: number) => factionNameAt(faction, hour, factionOverrides?.[faction.id]?.name), ideas: markerIdeas.data && markerIdeas.data.tag === markerTag ? markerIdeas.data : undefined,
    parties: preparedParties.enabled && preparedParties.summary?.marks.some(mark=>!mark.exact) ? undefined : markerParties.data, focus: markerFocus.data,
  }, [preparedParties.enabled, preparedParties.summary, track, factionOverrides, markerTag, extent[0], extent[1], resolved.membership, markerGroups.data, markerIdeas.data, markerParties.data, markerFocus.data]); // eslint-disable-line react-hooks/exhaustive-deps
  // Other countries are named as the game showed them at each event's hour, never by
  // the metadata's name from another date.
  const markerReads = useMemo(() => {
    const countries: NameAt[] = [], focuses: NameAt[] = [];
    if (markerSources) stabilityMarkers({ ...markerSources,
      countryName: (other, hour) => { countries.push({ hour, tag: other }); return ''; },
      focusName: (token, hour) => { focuses.push({ hour, tag: token }); return null; },
    }, kinds);
    // Every marker date stays admitted, so a long history never leaves later names unread.
    return { countries, focuses, nameHours: Math.max(MAX_NAME_HOURS, new Set(countries.map(item => item.hour)).size) };
  }, [markerSources, kinds]);
  const namesAt = useCountryNamesAt(runtime.client, track, markerReads.countries, active, markerReads.nameHours);
  const focusText = useFocusText(runtime.client, track, markerTag ?? undefined, markerReads.focuses, active);
  const badges = useMemo(() => !track || !markerSources ? [] : stabilityMarkers({ ...markerSources,
    countryName: (other, hour) => nameReadFailure(namesAt.get(nameKey(hour, other))) ?? nameReadText(namesAt.get(nameKey(hour, other))),
    focusName: (token, hour) => {
      const item = focusText.read.get(nameKey(hour, token));
      return isFailedFact(item) ? item : item === undefined ? undefined : item?.artwork.name ?? null;
    },
  }, kinds), [track, markerSources, namesAt, focusText.read, kinds]);
  // The marker sources shown report their notes and failures, so a missing kind of event is never silent.
  const markerNotes = useMemo(() => markerTag ? ([
    ['wars and factions', markerGroups, kinds.has('wars') || kinds.has('factions')], ['laws and advisors', markerIdeas, kinds.has('government')],
    ['ruling party', markerParties, kinds.has('government')], ['focuses', markerFocus, kinds.has('focus')],
  ] as const).flatMap(([what, state, used]) => {
    if (!used) return [];
    const product = state as { error?: Failure; data?: { warnings: string[] } };
    return product.error ? [`Markers for ${what} could not be loaded: ${product.error.message}`] : product.data?.warnings ?? [];
  }) : [], [markerTag, kinds, markerGroups.data, markerGroups.error, markerIdeas.data, markerIdeas.error, markerParties.data, markerParties.error, markerFocus.data, markerFocus.error]); // eslint-disable-line react-hooks/exhaustive-deps
  const drillAnchor = useRef<HTMLDivElement>(null);
  const singleTag = resolved.tags.length === 1 ? resolved.tags[0] : null;
  const choiceScope = useMemo(() => JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id,
    widget.countries, widget.countries.kind === 'selected' ? fallback : null, widget.time, extent, resolved.tags, resolved.membership ? [...resolved.membership] : null,
    options.stability, options.warSupport, options.scale, options.show, options.playback, options.afterPlayhead, options.windowDays]),
  [track?.source.id, track?.source.revision, track?.branch.id, widget.countries, widget.countries.kind === 'selected' ? fallback : null, widget.time, extent[0], extent[1], resolved.tags, resolved.membership,
    options.stability, options.warSupport, options.scale, options.show, options.playback, options.afterPlayhead, options.windowDays]);
  const valueOwner = prepared.enabled ? prepared.owner : numbers;
  const pickOwner = useMemo(() => ({ scope: choiceScope, numbers: valueOwner }), [choiceScope, valueOwner]);
  const chosen = actorPick?.owner === pickOwner ? actorPick.pick : null;
  const drillOpen = drill.breakdown !== null && chosen !== null;
  const drillTag = chosen ? chosen.tag : singleTag;
  const choosingActor = chosen !== null && chosen.tag === null;
  useEffect(() => { dispatchDrill({ type: 'close' }); setActorPick(null); }, [pickOwner]);
  const origin = useRef<{ owner: typeof bindingOwner; hour: number } | null>(null);
  if (origin.current?.owner !== bindingOwner) origin.current = null;
  if (pinnedMeasure && window && !origin.current) origin.current = { owner: bindingOwner, hour: window.lastHour };
  useEffect(() => setLocalPick(null), [bindingOwner]);
  const savedCountry = useMemo(() => readDrillCountry(widget.options?.stabilityCountry, track, widget.countries),
    [widget.options?.stabilityCountry, track, widget.countries]);
  const selectedCountry = useMemo(() => savedCountry ?? (hasSavedCountry ? null
    : groupData && track && drillTag ? chooseDrillCountry(groupData.countryIdentities, track, drillTag,
      chosen?.hour ?? origin.current?.hour ?? window?.lastHour ?? lastHour) : null),
    [savedCountry, hasSavedCountry, groupData, track, drillTag, chosen?.hour, bindingOwner]);
  const actorSpans = useMemo(() => selectedCountry && groupData && track ? drillCountrySpans(groupData.countryIdentities, selectedCountry, track, resolved.membership) : null,
    [selectedCountry, groupData, track, resolved.membership]);
  const aliasTags = useMemo(() => actorSpans ? [...actorSpans.keys()] : [], [actorSpans]);
  const partsRequests = track && !runtime.client.preparedQuery && (drillOpen || pinnedMeasure) ? aliasTags.map(tag => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'stability-parts', params: { branchId: track.branch.id, tag } })) : [];
  const parts = useProducts<StabilityPartsReply>(runtime.client, partsRequests, active, validateStabilityPartsReply);
  const joinedParts = useMemo(() => runtime.client.preparedQuery && track && actorSpans && drillTag && (drillOpen || pinnedMeasure)
    ? {branchId:track.branch.id,tag:drillTag,decimals:5,stability:{parts:[],observed:[]},warSupport:{parts:[],observed:[]},warnings:[]}
    : parts.data && actorSpans && drillTag ? stabilityCountryParts(parts.data, actorSpans, drillTag) : undefined,
    [parts.data, actorSpans, drillTag, runtime.client, track, drillOpen, pinnedMeasure]);
  const partsStore = useMemo(() => new NumericReply<StabilityPartsReply>(), [runtime.client]);
  const partsKey = JSON.stringify([partsRequests, selectedCountry, actorSpans ? [...actorSpans] : null]);
  const partNumbers = partsStore.update(partsKey, joinedParts, stabilityPartsNumbers);
  const partsData = joinedParts ?? partNumbers;
  const detailPickOwner = useMemo(() => ({ parent: pickOwner, actor: selectedCountry, measure: drillMeasure, parts: partNumbers }),
    [pickOwner, selectedCountry, drillMeasure, partNumbers]);
  const preparedPartsBinding = track ? {client:runtime.client,source:track.source,branchId:track.branch.id,tags:aliasTags,within:actorSpans ?? undefined} : undefined;
  const [preparedPartsNotes,setPreparedPartsNotes] = useState<readonly string[]>([]);
  const liveHour = track ? Math.floor(clockHour) + track.axisOffset : null;
  const pinnedFollowing = localHour === null || localHour === undefined && following;
  const pinnedAt = localHour === undefined ? playhead ?? extent[1] : localHour === null ? liveHour : localHour;
  const drillAt = pinnedMeasure ? pinnedAt : drillOpen ? drillHour(drill, liveHour) : null;
  const drillPlayhead = pinnedMeasure ? pinnedAt : drillOpen && drill.follow ? liveHour : playhead;
  const aliasPrepared = usePreparedStability(runtime.client, aliasTags.length ? track?.source : undefined, track?.branch.id, aliasTags, drillAt ?? extent[1], preparedHover, active);
  const aliasRequest = track && aliasTags.length ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-stability', params: { branchId: track.branch.id, tags: aliasTags, hour: null } } : null;
  const aliasValues = useProduct<CountryStabilityReply>(runtime.client, aliasRequest, active && !aliasPrepared.enabled, validateStabilityReply);
  const aliasStore = useMemo(() => new NumericReply<CountryStabilityReply>(), [runtime.client]);
  const aliasNumbers = aliasStore.update(JSON.stringify(aliasRequest), aliasPrepared.enabled ? aliasPrepared.data : aliasValues.data, stabilityNumbers);
  const aliasData = (aliasPrepared.enabled ? aliasPrepared.data : aliasValues.data) ?? aliasNumbers;
  const shownTag = actorSpans && drillAt !== null ? countryDrillTagAt(actorSpans, Math.floor(drillAt)) : null;
  const drillLabels = useCountryLabelsAt(runtime.client, track,
    (drillOpen || pinnedMeasure && savedCountry) && shownTag && drillAt !== null ? [{ tag: shownTag, hour: Math.floor(drillAt) }] : [], active);
  const drillReadAt = (hour: number) => {
    const tag = actorSpans ? countryDrillTagAt(actorSpans, hour) : null;
    if (aliasPrepared.enabled && tag && !aliasPrepared.hourLoaded(tag, hour)) { queueMicrotask(() => setPreparedHover(hour)); return null; }
    const carried = (aliasData ?? data)?.series.find(item => item.tag === tag)?.carried;
    return carried ? readHourAt(carried, hour) : null;
  };
  const actorReason = hasSavedCountry && !savedCountry ? 'Selected country unavailable.'
    : !groupData ? 'Loading country…'
      : !selectedCountry || !actorSpans ? 'Selected country unavailable.'
        : !shownTag ? 'Selected country unavailable.' : null;
  const countryHeader = pinnedMeasure && countrySlot ? createPortal(<DatedCountryChip hour={drillAt === null ? null : Math.floor(drillAt)}
    label={savedCountry && shownTag && drillAt !== null ? drillLabels.labelAt(Math.floor(drillAt))(shownTag) : null}
    reason={hasSavedCountry ? actorReason : 'Choose a country.'} />, countrySlot) : null;
  // Every diagnostic in one place, so no source's warning replaces another's.
  useEffect(() => {
    const uncoloured = series.filter(item => !item.recordedColour).map(item => item.name);
    const warnings = [...(data?.warnings ?? []),
      ...(error ? [`Stability values could not be refreshed: ${error.message}`] : []), ...(groups.data?.warnings ?? []),
      ...(groups.error ? [`Country membership could not be loaded: ${groups.error.message}`] : []),
      ...(uncoloured.length ? [`Map colours unavailable for ${uncoloured.join(', ')}; shown in grey.`] : []),
      ...(breakdown.data?.warnings ?? []), ...(breakdown.error ? [`The breakdown could not be loaded: ${breakdown.error.message}`] : []),
      ...markerNotes, ...focusText.warnings, ...preparedPartsNotes,
      ...(partsRequests.length ? [...(joinedParts?.warnings ?? []), ...(parts.error ? [`The parts could not be loaded: ${parts.error.message}`] : [])] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, error, series, groups.data, groups.error, breakdown.data, breakdown.error, markerNotes, focusText.warnings, partsRequests.length, joinedParts, parts.error, preparedPartsNotes, onDiagnostics]);
  const menu = useContextMenu('Chart actions');
  const head = playhead ?? extent[1];
  // With the keyboard: while focus is in the widget, the breakdown of the read behind
  // the moment shown is loaded once playback rests there, and read out below.
  const [focused, setFocused] = useState(false);
  // Only while the country is shown there (a group counted at each hour may have left it).
  // ... and only where the hour shown was itself recorded (a read can cover an hour left out).
  const headRecorded = single != null && shown.some(measure => rawAt(single[measure.key], Math.floor(head)) != null);
  const headRead = single && headRecorded && isMember(resolved.membership?.get(single.tag), Math.floor(head)) ? readHourAt(single.carried, Math.floor(head)) : null;
  // Its own request, so a pointer resting on the chart never replaces it.
  const [spokenAt, setSpokenAt] = useState<{ tag: string; read: number; hour: number } | null>(null);
  // A new recording, branch or country asks afresh.
  useEffect(() => setSpokenAt(null), [track?.source.id, track?.source.revision, track?.branch.id, single?.tag]);
  const spokenRequest = track && focused && single && spokenAt && spokenAt.tag === single.tag ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-stability', params: { branchId: track.branch.id, tags: [single.tag], hour: spokenAt.hour, breakdownOnly: true } } : null;
  const spoken = useProduct<CountryStabilityReply>(runtime.client, spokenRequest, active, validateStabilityReply);
  const spokenReply = spoken.data?.series[0];
  const spokenFor = single && spokenReply?.tag === single.tag && spokenReply.breakdown?.readHour === headRead ? spokenReply.breakdown : null;
  // An hour of the read that had no breakdown (a gap) is asked again at the hour shown.
  const spokenEmpty = spoken.data != null && spokenReply?.tag === single?.tag && (spokenReply?.breakdown == null || !spokenReply.breakdown.linesAvailable);
  useEffect(() => {
    if (!focused || !single || headRead == null) return;
    const asked = spokenAt?.tag === single.tag && spokenAt.read === headRead;
    if (asked && !(spokenEmpty && spokenAt.hour !== Math.floor(head))) return;
    const timer = setTimeout(() => setSpokenAt({ tag: single.tag, read: headRead, hour: Math.floor(head) }), FOCUS_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [focused, single, headRead, head, spokenAt, spokenEmpty]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const chartParity = import.meta.env.MODE === 'parity' ? usePreparedScalarParity(runtime.client, track?.source, track?.branch.id, resolved.tags, extent, resolved.membership ?? undefined) : undefined;
  const layers = useMemo<ChartLayer[]>(() => series.flatMap(item => shown.map(measure => ({
    draw: (ctx, frame, chartTokens, state) => {
      const summary = prepared[measure.key].summaryByTag.get(item.tag)?.summary;
      if (prepared.enabled) { if (summary) drawLineSummary(ctx, frame, chartTokens, percentSummary(summary), item.hex, false, state.playhead, { dash: measure.dash }); }
      else drawLine(ctx, frame, chartTokens, item.lines[measure.key], item.hex, { dash: measure.dash, until: state.playhead ?? undefined });
    },
    extent: (x: readonly [number, number]) => {
      if (!prepared.enabled) return extentOf(item.lines[measure.key], x[0], Math.min(x[1], until ?? x[1]));
      const summary = prepared[measure.key].summaryByTag.get(item.tag)?.summary;
      const buckets = summary && summary.to <= Math.min(x[1], until ?? x[1]) && summary.to >= x[0] ? summary.buckets.filter(bucket => bucket !== null) : [];
      return buckets.length ? [Math.min(...buckets.map(bucket => bucket.minValue * 100)), Math.max(...buckets.map(bucket => bucket.maxValue * 100))] : null;
    },
  }))), [series, shown, until, prepared.enabled, prepared.stability.summaryByTag, prepared.warSupport.summaryByTag]);

  if (!track) return <WidgetState kind="empty" icon="sliders" message="Load a campaign to see stability and war support." />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (widget.countries.kind === 'group' && groups.error && !groupData) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  const groupNotice = widget.countries.kind === 'group' && groups.error && groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (resolved.reason) return <>{groupNotice}<WidgetState kind="empty" message={resolved.reason} /></>;
  // A group's members at hours still being prepared are not known yet: none is not proof of none.
  const preparing = <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  const branchKnown = resolved.known(track.branch.firstHour, track.branch.lastHour);
  // A pinned breakdown is the widget itself: the main chart's settings do not apply.
  if (pinnedMeasure) {
    const body = !singleTag ? branchKnown ? <WidgetState kind="empty" message="Choose one country to see what makes up its values." /> : preparing
      : (hasSavedCountry && !savedCountry) || (groupData && (!selectedCountry || !actorSpans))
        ? <WidgetState kind="unavailable" message={actorReason!} />
        : groups.error && !groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
          : parts.error && !partsData ? <WidgetState kind="error" failure={parts.error} onRetry={parts.retry} />
            : error && !data ? <WidgetState kind="error" failure={error} onRetry={retry} />
              : !partsData || !data ? <WidgetState kind="loading" label="Loading what makes up the values…" />
                : <StabilityDrill preparedBinding={preparedPartsBinding} onPreparedDiagnostics={setPreparedPartsNotes} reply={partsData} numbers={partNumbers!} pickOwner={detailPickOwner} presentationReady={parts.data !== undefined} measure={pinnedMeasure}
                  playback={{ playhead: drillPlayhead, mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }} hour={drillAt} extent={extent}
                  within={null} readAt={drillReadAt} onPick={hour => setLocalHour(Math.floor(hour))}
                  onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])}
                  branchStart={track.branch.firstHour} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} tokens={tokens} />;
    return <div className="stability is-pinned">
      {countryHeader}
      {drillAt !== null && <p className="chart-line-key">{formatDateTime(Math.floor(drillAt))} {!pinnedFollowing && <button type="button" className="button small"
        onClick={() => setLocalHour(null)}>Follow playback</button>}</p>}
      {body}
      {parts.error && <WidgetState kind="error" what="the breakdown" failure={parts.error} onRetry={parts.retry} />}
      {groups.error && groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
      {error && <WidgetState kind="error" what="values" failure={error} onRetry={retry} />}
      {menu.menu}
    </div>;
  }
  if (error && !data) return <>{groupNotice}<WidgetState kind="error" failure={error} onRetry={retry} /></>;
  if (!data || (widget.countries.kind === 'group' && !groupData)) return <>{groupNotice}<WidgetState kind="loading" label="Loading stability and war support…" /></>;
  if (!shown.length) return <>{groupNotice}<WidgetState kind="empty" message="Both lines are switched off in the settings." /></>;
  if (!prepared.enabled && series.every(item => shown.every(measure => !item.lines[measure.key].length))) return <>{groupNotice}{branchKnown
    ? <WidgetState kind="unavailable" message="Stability and war support are unavailable for these countries." /> : preparing}</>;

  const decimals = data.decimals;
  // Zoom fits the values drawn in view (both bounds, as the view moves); the fixed scale is 0-100%.
  const zoom = options.scale === 'zoom';
  // The breakdown of the moment shown, every line, when it has been loaded for the keyboard.
  const spokenBreakdown = spokenFor ? shown.map(measure =>
    breakdownRows(spokenFor, measure.key, decimals, measure.label, Number.MAX_SAFE_INTEGER).rows
      .map(row => row.value ? `${row.label} ${row.value}` : row.label).join(', ')).join('; ') : '';
  const spokenNote = !focused || headRead == null ? ''
    : spokenBreakdown ? `Breakdown on ${dayLabel(Math.floor(head))}${headRead < Math.floor(head) ? `, as of ${formatDateTime(headRead)}` : ''}: ${spokenBreakdown}.`
      : spoken.error && spokenAt?.read === headRead ? failureText(spoken.error, 'Couldn’t load the breakdown.')
        : spokenEmpty && spokenAt?.read === headRead && spokenAt.hour === Math.floor(head) ? `No breakdown was recorded for ${dayLabel(Math.floor(head))}.` : '';
  const main = <div className="stability-main" ref={chartParity}>
    {partyParity && <div ref={partyParity} />}
    <Legend label="Countries" items={legendItems} windowed onVisibleItems={onVisibleItems} />
    {/* The line styles are named: never the dash pattern alone. */}
    <p className="stability-key">{shown.map(measure => <span key={measure.key}><i aria-hidden="true" className={measure.dash ? 'is-dashed' : undefined} />{measure.dash ? 'Dashed' : 'Solid'}: {measure.label.toLowerCase()}</span>)}</p>
    {/* The current values as text, for keyboard and screen-reader users. */}
    <p className="sr-only" aria-live="off">{`${dayLabel(Math.floor(head))}: `}{series.map(item => {
      // Values carried from an earlier read say when they were read, as the tooltip does.
      const read = readHourAt(item.carried, Math.floor(head));
      const values = shown.map(measure => {
        const raw = item.valueAt(measure.key, Math.floor(head));
        return `${measure.label} ${raw == null ? 'not recorded' : formatPercent(raw, decimals)}`;
      });
      const carried = read < Math.floor(head) && shown.some(measure => item.valueAt(measure.key, Math.floor(head)) != null);
      return `${item.name}: ${values.join(', ')}${carried ? ` (as of ${formatDateTime(read)})` : ''}`;
    }).join('; ')}</p>
    {/* Announced when it loads for a keyboard user, as focus stays on the chart. */}
    <p className="sr-only" aria-live="polite" data-detail={spoken.error?.message}>{spokenNote}</p>
    {kinds.has('government') && <PartyEventList prepared={preparedParties} badges={partyPageBadges} />}
    {/* Leaving the chart ends a hover, so coming back to the same hour asks again for a failed breakdown. */}
    <div className="stability-chart" onPointerLeave={() => { tipHour.current = null; }}>
      <TimeChart label={`Stability and war support of ${series.map(item => item.name).join(', ')}`} extent={extent} playhead={playhead}
        onSummaryView={prepared.enabled || preparedParties.enabled ? view=>{if(prepared.enabled)prepared.onSummaryView(view);if(preparedParties.enabled)preparedParties.onSummaryView(view);} : undefined}
        onNavigateHour={prepared.enabled ? prepared.onNavigateHour : undefined} keyboardHour={prepared.keyboardHour} keyboardRevision={prepared.keyboardRevision}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: zoom ? null : 0, yMax: zoom ? null : 100, log: false }} fitValues={zoom ? zoomRange : undefined}
        padding={{ right: options.endLabels ? CHART_LAYOUT.countryEndLabels : CHART_LAYOUT.right }} layers={layers} active={active && !(drillOpen && options.drill !== 'popout')}
        pickOwner={pickOwner} revision={`${head}:${looks.revision}:${drillLabels.revision}:${data.branchId}:${series.length}:${shown.length}:${breakdown.data ? breakdownFor?.readHour ?? 'empty' : breakdown.error ? 'failed' : 'none'}`}
        formatValue={value => `${Number.isInteger(value) ? value : value.toFixed(1)}%`}
        onHover={preparedParties.enabled ? setPartyHover : undefined}
        markSummaries={kinds.has('government') && preparedParties.enabled ? partyMarkSummaries(preparedParties.summary,partyEvents(preparedParties.summary?.reply))?.map(summary=>({...summary,clusters:summary.clusters.map(cluster=>({...cluster,badge:cluster.badge ? {...cluster.badge,glyph:'\u2699'} : undefined}))})) : undefined}
        onMarkHover={kinds.has('government') && preparedParties.enabled ? preparedParties.onMarkHover : undefined}
        badgesAtHour={kinds.has('government') && preparedParties.enabled ? hour=>[...badges.filter(badge=>badge.hour===hour),...partyReachedBadges.filter(badge=>badge.hour===hour)] : undefined}
        markTooltip={cluster=>({title:dayLabel(cluster.firstHour),rows:preparedParties.events ? badgeRows(partyPageBadges) : [{label:'Events',value:'Loading...'}]})}
        badges={badges.length ? badges : undefined}
        tooltip={(hour, _frame, hovered) => {
          if (prepared.enabled && preparedHover !== hour) queueMicrotask(() => setPreparedHover(hour));
          const rows: TooltipRow[] = [...badgeRows(hovered)];
          if (series.length === 1) {
            const item = series[0];
            const read = readHourAt(item.carried, hour);
            // Ask for the breakdown of the read behind this hour; the chart redraws when it arrives.
            const key = `${item.tag}:${read}`;
            const asked = requestHours.current.get(key);
            // A read whose asked hour was a gap is asked again at this (shown) hour.
            const retry = asked != null && asked !== hour && emptyHours.current.has(`${key}:${asked}`) && shown.some(measure => item.valueAt(measure.key, hour) != null);
            if (asked == null || retry) requestHours.current.set(key, hour);
            if (hoverKey.current !== key || retry) {
              hoverKey.current = key;
              const at = requestHours.current.get(key)!;
              queueMicrotask(() => setHover({ tag: item.tag, read, hour: at }));
            }
            // Once the attempts are spent, moving to another hour of the read asks afresh
            // (a tooltip rebuilt for the same hour is no new hover).
            const moved = tipHour.current !== hour;
            tipHour.current = hour;
            if (moved && breakdown.error && hoverRead === read && retries.current >= BREAKDOWN_RETRIES) {
              retries.current = 0;
              queueMicrotask(retryBreakdown.current);
            }
            // The reply for this read: loaded (with or without a breakdown), failed, or on its way.
            const current = breakdown.data && hoverRead === read;
            const ready = current && breakdownFor ? breakdownFor : null;
            for (const [index, measure] of shown.entries()) {
              const raw = item.valueAt(measure.key, hour);
              if (raw == null) { rows.push({ label: measure.label, value: 'Not recorded', tone: 'muted', strong: true, divider: index > 0 || rows.length > 0 }); continue; }
              if (!ready) {
                rows.push({ label: measure.label, value: formatPercent(raw, decimals), strong: true, divider: index > 0 || rows.length > 0 });
                continue;
              }
              const built = breakdownRows(ready, measure.key, decimals, measure.label, linesPerMeasure(shown.length));
              rows.push(...built.rows.map((row, position) => position === 0 ? { ...row, divider: index > 0 || rows.length > 0 } : row));
            }
            const notes = [
              ...(breakdown.error && hoverRead === read ? [failureText(breakdown.error, 'Couldn’t load the breakdown.')]
                : current && !breakdownFor ? ['No breakdown was recorded for this hour.']
                  : !ready ? ['Loading the breakdown…'] : []),
              // Values carried from an earlier read say when they were read.
              ...(read < hour ? [`As of ${formatDateTime(read)}.`] : []),
            ];
            return fitTooltip([{ title: dayLabel(hour), rows, detail: hoverRead === read ? breakdown.error?.message : undefined, note: notes.length ? notes.join(' ') : undefined }], viewport);
          }
          // All actors remain in the full tooltip; the shared fitter counts rows hidden visually.
          const carried: number[] = [];
          for (const item of series) {
            const values = shown.map(measure => {
              const raw = item.valueAt(measure.key, hour);
              return `${measure.dash ? 'WS' : 'S'} ${raw == null ? '—' : formatPercent(raw, decimals)}`;
            });
            const none = shown.every(measure => item.valueAt(measure.key, hour) == null);
            const read = readHourAt(item.carried, hour);
            const early = read < hour && !none;
            if (early) carried.push(read);
            rows.push({ label: item.name, swatch: item.hex, value: none ? 'Not recorded' : `${values.join(' · ')}${early ? ' †' : ''}`, tone: none ? 'muted' : undefined, short: true });
          }
          // Values carried from an earlier read are marked; the note stays one line however many there are.
          const first = carried.reduce((minimum, read) => Math.min(minimum, read), Infinity);
          const last = carried.reduce((maximum, read) => Math.max(maximum, read), -Infinity);
          const note = [`${shown.map(measure => `${measure.dash ? 'WS' : 'S'}: ${measure.label.toLowerCase()}`).join(', ')}.`,
            ...(carried.length ? [`† As of ${formatDateTime(first)}${first === last ? '' : ` – ${formatDateTime(last)}`}.`] : [])];
          return fitTooltip([{ title: dayLabel(hour), rows, note: note.join(' ') }], viewport);
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          const at = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          // No line ends in view (the view lies wholly after them): no labels.
          if (at < frame.x[0]) return;
          const ends = series.flatMap(item => shown.flatMap(measure => {
            // Each line's own last sample in view: a line may end before the others.
            const last = lineEndHour(item.lines[measure.key], frame.x[0], at);
            if (last == null) return [];
            const raw = item.valueAt(measure.key, last);
            if (raw == null) return [];
            const text = `${series.length > 1 ? `${item.name}: ` : ''}${measure.label} ${formatPercent(raw, decimals)}`;
            return [{ id: `${item.tag}:${measure.key}`, y: frame.toY(Number(raw) / 10 ** decimals * 100), text, color: item.hex }];
          }));
          // Only as many labels as the plot can hold: war support's are dropped first, then all
          // (the legend and tooltip still name every line).
          const room = Math.floor((frame.plotHeight + END_LABEL_GAP) / (END_LABEL_HEIGHT + END_LABEL_GAP));
          const fitted = ends.length <= room ? ends : ends.filter(end => end.id.endsWith(':stability'));
          if (fitted.length > room) return;
          ends.splice(0, ends.length, ...fitted);
          placeEndLabels(ctx, frame, chartTokens, ends, END_LABEL_GAP);
        }}
        onPick={async (hour, context?: ChartPick) => {
          let picked: StabilityPick | null;
          if (prepared.enabled) {
            const exact = await Promise.all(shown.map(measure => prepared[measure.key].exactAt(hour)));
            const known = new Set(exact.flatMap((reply, index) => reply?.states.filter(state => state.run && isMember(resolved.membership?.get(state.tag), hour)).map(state => JSON.stringify([state.tag, shown[index].key])) ?? []));
            const lines = series.flatMap(item => shown.flatMap(measure => { const summary = prepared[measure.key].summaryByTag.get(item.tag)?.summary; return summary ? [{ key: JSON.stringify([item.tag, measure.key]), summary: percentSummary(summary) }] : []; }));
            const witnessed = pickSummaryLines(lines, hour, known, context).map(key => { const [tag, measure] = JSON.parse(key) as [string, Measure]; return { tag, measure }; });
            const candidates = [...new Set(witnessed.map(line => line.tag))];
            picked = candidates.length ? { hour: Math.floor(hour), candidates, lines: witnessed, tag: candidates.length === 1 ? candidates[0] : null, measure: witnessed.find(line => line.measure === drillMeasure)?.measure ?? witnessed[0].measure } : null;
          } else picked = pickStabilityLine(series.flatMap(item => shown.map(measure => ({ tag: item.tag, measure: measure.key, runs: item.lines[measure.key] }))), hour, context, drillMeasure);
          if (!picked) return;
          setActorPick({ owner: pickOwner, pick: picked });
          setDrillMeasure(picked.measure);
          dispatchDrill(stabilityPickAction(picked, drillOpen, widget.time.mode, runtime.clock.getSnapshot().playing, playhead));
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {error && <WidgetState kind="error" what="values" failure={error} onRetry={retry} />}
  </div>;
  const breakdownOf = STABILITY_BREAKDOWNS.find(item => item.id === drillMeasure) ?? STABILITY_BREAKDOWNS[0];
  return <div className="stability" ref={drillAnchor} onFocus={() => setFocused(true)}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false); }}>
    {/* Outside the drill, so an open breakdown never hides a later failure. */}
    {groupNotice}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={drillAnchor}
      title={`${shownTag && drillAt !== null ? `${drillLabels.labelAt(Math.floor(drillAt))(shownTag).name}: ` : ''}What makes up ${MEASURE_NAMES[drillMeasure].toLowerCase()}`} day={drillAt == null ? '' : dayLabel(Math.floor(drillAt))}
      following={drill.follow} canFollow={!choosingActor && drillTag !== null && liveHour !== null && liveHour >= track.branch.firstHour && liveHour <= track.branch.lastHour}
      returnFocus=".stability-chart canvas" onClose={() => {
        dispatchDrill({ type: 'close' });
        setActorPick(null);
      }} onFollow={() => { if (!choosingActor && drillTag) dispatchDrill({ type: 'follow' }); }}
      onPin={selectedCountry && actorSpans && drillAt !== null ? () => runtime.addWidgetBelow(widget.id,
        pinStabilityDrill({ ...widget, options: { ...widget.options, stabilityCountry: JSON.stringify(selectedCountry) } }, breakdownOf, track.id, track.branch.firstHour, selectedCountry!.tag, drillAt!, drill.follow)) : undefined} main={main}>
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
        : parts.error && !partsData ? <WidgetState kind="error" failure={parts.error} onRetry={parts.retry} />
        : !partsData ? <WidgetState kind="loading" label="Loading what makes up the values…" />
          : <StabilityDrill preparedBinding={preparedPartsBinding} onPreparedDiagnostics={setPreparedPartsNotes} reply={partsData} numbers={partNumbers!} pickOwner={detailPickOwner} presentationReady={parts.data !== undefined} measure={drillMeasure} onMeasure={setDrillMeasure} playback={{ playhead: drillPlayhead, mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }} hour={drillAt} onPick={hour => dispatchDrill({ type: 'pick', hour })}
            within={null} readAt={drillReadAt}
            onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])}
            extent={extent} branchStart={track.branch.firstHour} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} tokens={tokens} />}
      {parts.error && partsData && <WidgetState kind="error" what="the breakdown" failure={parts.error} onRetry={parts.retry} />}
    </DrillHost>
    {menu.menu}
  </div>;
}
