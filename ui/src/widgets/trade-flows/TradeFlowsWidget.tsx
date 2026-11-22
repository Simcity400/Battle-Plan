import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { ReactNode } from 'react';
import { nameReadFrom, nameReadMetadata } from '../../dashboard/nameReads.ts';
import { useCallback, useEffect, useMemo,  useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { dayLabel } from '../../charts/draw';
import FlowDiagram from '../../charts/flow/FlowDiagram';
import type { FlowHit, FlowLink, FlowNode, FlowValues } from '../../charts/flow/FlowDiagram';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { liveAt } from '../../charts/live';
import LiveText, { liveTexts, separated } from '../../charts/LiveText';
import { usNumber } from '../../charts/numberFormat';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount } from '../../charts/valueAxis';
import { MAX_NAME_HOURS, isFailedFact } from '../../dashboard/countryFacts.ts';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { useContentRevision } from '../../dashboard/contentRevision';
import { flagReadMetadata, useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryFlagsRead } from '../../dashboard/countryFlags';
import { LOOK_STEP, lookHour } from '../../dashboard/countryLooks';
import { nameKey, useCountryFactsByKey } from '../../dashboard/countryNamesAt';
import type { NameAt } from '../../dashboard/countryNamesAt';
import { STYLES } from '../../dashboard/useCountriesAt';
import { sampledLooks } from '../../dashboard/looks';
import type { Look } from '../../dashboard/looks';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import type { WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import { noMembersAt, worldFilter } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions } from '../../dashboard/options';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime';
import {  recordedHour } from '../../dashboard/regionTime';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import type { Gate } from '../../dashboard/regionGate';
import { perfLog } from '../../shell/perf.ts';
import { mix, toHex } from '../../shell/appearance/color';
import { buildDiagram, campaignScale, coverageNote, sideRanks, decodeTrade, exactSums, formatDays, knownHours, emptyMessage, knownSpanAt, unrecordedMessage,
  measureAmount, measureNote, OTHER, resourceColours, selectionMessage, identityCountryAt, resourceName, selectFlows } from './tradeData.ts';
import type { Flow, Measure, SideNode, SideLink, TradeData } from './tradeData';
import TradeDrill, { tradeTexts } from './TradeDrillView';
import type { LiveHour } from './TradeDrillView';
import TradeCountryView, { countryTexts, resourceLegendItems } from './TradeCountryView';
import { countryTradeTags } from './tradeCountry';
import { contributionCountryTag, linkSelection, nodeSelections, observedTradeSpans, pinContributions, readSelection, selectedContributors, tradeContext } from './tradeDrill';
import type { TradeSelection } from './tradeDrill';
import { heldTradeColours, tradeColourPlan, tradeCountryColours, tradeFixedColourKey } from './tradeColours';
import { OPTIONS } from './definition';
import { useTradeFlows, useTradeWindows } from './useTradeFlows';
import { nodeTooltip, ribbonTooltip } from './tradeTooltip';
import { marketLendLeaseWaits, validateMarketReply } from './marketData';
import './tradeFlows.css';
import TradeMarketView from './TradeMarketView';
import MarketOccurrences from '../../dashboard/MarketOccurrences';
import { DELIVERY_KINDS } from '../../dashboard/marketLedger';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];
const DESCRIBED = 24;

type Label = CountryFlagLook & { name: string; color: readonly [number, number, number] | null };
type TradeNode = FlowNode & { trade: { selection: TradeSelection; node: SideNode; links: readonly SideLink[]; hour: number; from: number } };
type TradeLink = FlowLink & { trade: { selection: TradeSelection; link: SideLink; source: SideNode; target: SideNode; hour: number; from: number } };
const ALL_CONTRIBUTORS: TradeSelection = { kind: 'all' };

function useLookHour(runtime: WidgetRuntime, enabled: boolean, origin: number): number {
  const held = useRef<{ step: number; hour: number } | null>(null);
  const subscribe = useCallback((changed: () => void) => {
    if (!enabled) return () => {};
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock, enabled]);
  const read = () => {
    if (!enabled) return -1;
    const hour = runtime.clock.getHour(), step = Math.floor((hour - origin) / LOOK_STEP);
    if (held.current?.step !== step) held.current = { step, hour };
    return step;
  };
  useSyncExternalStore(subscribe, read, read);
  return enabled && held.current ? held.current.hour : runtime.clock.getHour();
}

function marketSelectionAt(members: ReturnType<typeof useCountriesSelectionAt>, on: boolean, all: boolean) {
  if (!on) return { tags: [], reason: null, ready: false, outside: false };
  if (members.outside) return { tags: [], reason: null, ready: false, outside: true };
  const tags = members.countries ? [...members.countries].sort() : [];
  const reason = marketLendLeaseWaits() ?? members.selectionReason
    ?? (!all && members.group.data && members.known && !tags.length ? 'Selected countries are unavailable at this time.' : null);
  return { tags, reason, ready: !reason && (all || !!members.group.data && tags.length > 0), outside: false };
}

export default function TradeFlowsWidget({ widget, runtime, active, countrySlot, onDiagnostics, setOption }: WidgetProps) {
  const drillControl = useDrill(runtime, useRegion(widget), widget.options?.show === 'contributions' ? 'contributions' : null);
  const drill = drillControl.state;
  const dispatchDrill = drillControl.dispatch;
  const [selection, setSelection] = useState<TradeSelection | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  const options = resolveOptions(OPTIONS, widget.options);
  const marketView = options.view === 'market';
  const pinnedView = options.show === 'contributions';
  const [marketSelection, setMarketSelection] = useState<string | null>(null);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const measure = (options.measure as Measure) ?? 'current';
  const asked = Number(options.perSide ?? 12);
  const [capacity, setCapacity] = useState<number | null>(null);
  const perSide = capacity == null ? asked : Math.max(1, Math.min(asked, capacity - 1));
  const incoming = useTradeFlows(widget, runtime, active);
  const { track, all, resolved, groups, data, scope, retry, extent } = incoming;
  const error = useHeldFailure(incoming.error, data !== undefined, retry);
  const following = widget.time.mode === 'follow';
  const axisOffset = track?.axisOffset ?? 0;
  const lookHourNow = useLookHour(runtime, active && !!track && (following || drill.breakdown !== null && drill.follow), track ? track.branch.firstHour - axisOffset : 0);
  const window = track && resolveWindow(widget, track, runtime.tracks, lookHourNow);
  const clampHour = useCallback((hour: number) => Math.min(extent[1], Math.max(extent[0], Math.floor(hour))), [extent[0], extent[1]]);
  const headAt = useCallback((position: number) => clampHour(position + axisOffset), [clampHour, axisOffset]);
  const playhead = window && (following || widget.time.mode === 'pin') ? Math.floor(window.lastHour) : null;
  const moment = clampHour(playhead ?? extent[1]);
  const from = extent[0];
  const menu = useContextMenu('Trade actions');

  const trade = useMemo(() => data && decodeTrade(data), [data]);
  const identities = useMemo(() => data ? countryIdentityIndex(data.countryIdentities.map(country => ({ tag: country.tag, identities: country.runs }))) : null, [data]);
  const chosen = contentSet(options.resources);
  const chosenKey = JSON.stringify(chosen);
  const resourceIds = useMemo(() => {
    if (!trade || !chosen.length) return null;
    return new Set(trade.reply.resources.filter(item => chosen.includes(item.token)).map(item => item.id));
  }, [trade, chosenKey]);
  const flows = useMemo(() => trade && scope ? selectFlows(trade, scope, resourceIds) : [], [trade, scope, resourceIds]);
  const scale = useMemo(() => options.scale !== 'moment' ? campaignScale(flows, measure, from, extent[1]) : null, [options.scale, flows, measure, from, extent[1]]);
  const ranks = useMemo(() => sideRanks(flows, extent[0], extent[1]), [flows, extent[0], extent[1]]);
  const fallback = defaultCountryKey(track);

  const lookFacts = useMemo(() => {
    const facts = new Map<string, NameAt & { stretch: number }>();
    if (trade && track) for (const flow of flows) for (const identity of [flow.sellerIdentity, flow.buyerIdentity]) {
      if (facts.has(identity)) continue;
      const fact = identityCountryAt(trade, identity, moment);
      const sample = fact && lookHour(track.branch, fact.hour, fact.firstHour);
      if (fact && sample) facts.set(identity, { tag: fact.tag, hour: sample.hour, stretch: sample.stretch });
    }
    return facts;
  }, [trade, track, flows, moment]);
  const looksWanted = useMemo(() => [...lookFacts.values()], [lookFacts]);
  const lookHours = Math.max(MAX_NAME_HOURS, new Set(looksWanted.map(item => item.hour)).size);
  const fixed = tokens.countryColours === 'fixed';
  const colourPlan = useMemo(() => tradeColourPlan(trade ?? null, track?.branch, moment, fixed), [trade, track?.branch, moment, fixed]);
  const styles = useCountryFactsByKey(runtime.client, track, fixed ? colourPlan.wanted : looksWanted, active, STYLES, fixed ? colourPlan.maxHours : lookHours);
  const flags = useCountryFlagsRead(runtime.client, track, looksWanted, active);
  const contentRevision = useContentRevision(runtime.client);
  const heldKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, contentRevision]);
  const held = useRef({ key: heldKey, client: runtime.client, looks: new Map<string, Look>(), segments: new Map<string, number>() });
  if (held.current.key !== heldKey || held.current.client !== runtime.client) held.current = { key: heldKey, client: runtime.client, looks: new Map(), segments: new Map() };
  const heldLooks = useMemo(() => {
    const samples = [...lookFacts].map(([identity, fact]) => ({ tag: identity, hour: fact.hour, stretch: fact.stretch }));
    const next = sampledLooks(held.current, samples, sample => {
      const fact = lookFacts.get(sample.tag)!;
      const read = flags.get(nameKey(fact.hour, fact.tag));
      return { style: styles.get(nameKey(fact.hour, fact.tag)), flag: isFailedFact(read) ? read : read === undefined ? undefined : read?.url ?? null };
    });
    held.current.looks = next.looks; held.current.segments = next.segments;
    return next.looks;
  }, [lookFacts, styles, flags, heldKey, runtime.client]);
  const fixedKey = useMemo(() => fixed ? tradeFixedColourKey(colourPlan, styles) : null, [fixed, colourPlan, styles]);
  const correctedColours = useMemo(() => fixedKey == null ? null : tradeCountryColours(fixedKey, tokens.surface, true), [fixedKey, tokens.surface]);
  const colourContext = JSON.stringify([heldKey, tokens.surface]);
  const heldColours = useRef({ key: colourContext, client: runtime.client, colours: new Map<string, string>(), selection: colourPlan.selection });
  if (heldColours.current.key !== colourContext || heldColours.current.client !== runtime.client) heldColours.current = { key: colourContext, client: runtime.client, colours: new Map(), selection: colourPlan.selection };
  const fixedColours = useMemo(() => {
    const colours = correctedColours ?? heldTradeColours(heldColours.current.colours, heldColours.current.selection, colourPlan.selection);
    heldColours.current.colours = colours; heldColours.current.selection = colourPlan.selection;
    return colours;
  }, [correctedColours, colourPlan, colourContext, runtime.client]);
  const labelOf = useCallback((identity: string): Label => {
    const fact = lookFacts.get(identity);
    const look = heldLooks.get(identity);
    const pending = active && !!track && !!fact && (!fixed || colourPlan.wanted.some(item => item.hour === fact.hour && item.tag === fact.tag)) && !styles.has(nameKey(fact.hour, fact.tag));
    return { ...nameReadMetadata(nameReadFrom(look?.nameFailure ?? look?.name ?? (pending ? undefined : null), pending)),
      ...(fact ? flagReadMetadata(flags.get(nameKey(fact.hour, fact.tag))) : {}), flag: look?.flag ?? undefined, color: look?.color ?? null };
  }, [lookFacts, heldLooks, flags, active, track, styles, fixed, colourPlan]);
  const [hoverWanted, setHoverWanted] = useState<readonly NameAt[]>([]);
  const hoverKey = useRef('');
  const askHover = useCallback((wanted: readonly NameAt[]) => {
    const unique = new Map(wanted.map(item => [nameKey(item.hour, item.tag), item]));
    const sorted = [...unique.values()].sort((a, b) => a.hour - b.hour || a.tag.localeCompare(b.tag));
    const key = JSON.stringify(sorted);
    if (key === hoverKey.current) return;
    hoverKey.current = key;
    queueMicrotask(() => setHoverWanted(sorted));
  }, []);
  const hoverLabels = useCountryLabelsAt(runtime.client, track, hoverWanted, active, Math.max(MAX_NAME_HOURS, new Set(hoverWanted.map(item => item.hour)).size));
  useEffect(() => {
    dispatchDrill({ type: 'close' }); setSelection(null); setMarketSelection(null); hoverKey.current = ''; setHoverWanted([]);
  }, [track?.source.id, track?.source.revision, track?.branch.id, options.view]);
  useEffect(() => { if (!active) { hoverKey.current = ''; setHoverWanted([]); } }, [active]);

  const shownResources = useMemo(() => {
    const ids = new Set(flows.map(flow => flow.resource.id));
    return trade ? trade.reply.resources.filter(item => ids.has(item.id)) : [];
  }, [flows, trade]);
  const otherColour = mix(tokens.surface, tokens.text, 0.4);
  const resourceColour = useMemo(() => trade ? resourceColours(trade.reply.resources, trade.resourceOrder, tokens.categories, otherColour) : new Map<number, string>(),
    [trade, tokens.categories, otherColour]);
  const resourceLegend = useMemo(() => resourceLegendItems(shownResources, resourceColour, otherColour), [shownResources, resourceColour, otherColour]);
  const nodeName = useCallback((node: SideNode) => node.identity ? labelOf(node.identity).name
    : `${usNumber.format(node.members.length)} other ${node.side === 'source' ? (node.members.length === 1 ? 'seller' : 'sellers') : (node.members.length === 1 ? 'buyer' : 'buyers')}`, [labelOf]);

  const lastHour = track?.branch.lastHour ?? 0;
  const groupUnobserved = widget.countries.kind === 'group' && widget.countries.membership === 'hour' ? resolved?.unobserved ?? null : null;
  const messageAt = useCallback((hour: number, drawn: number): string | null => {
    if (!data) return null;
    const unrecorded = unrecordedMessage(data.known, measure, from, hour);
    if (unrecorded) return unrecorded;
    const atMoment = worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, hour, lastHour, identities);
    const momentReason = atMoment.reason && (measure === 'current' || !resolved?.membership) ? atMoment.reason : null;
    if (momentReason) return momentReason;
    if (drawn) return null;
    return scope?.kind === 'countries' && scope.identityNote ? scope.identityNote : emptyMessage(data, measure, from, hour, groupUnobserved);
  }, [data, measure, from, widget.countries, fallback, groups.data, lastHour, identities, resolved, scope, groupUnobserved]);
  const coverageAt = useCallback((hour: number) => {
    if (!data) return '';
    const groupHours = groupUnobserved ? knownHours(groupUnobserved, from, hour) : 0;
    return [coverageNote(data, measure, from, hour), scope?.kind === 'countries' ? scope.identityNote : undefined,
      measure !== 'current' && groupHours >= 1 ? `${formatDays(groupHours)} of the group’s membership in this time were not recorded; they are not counted.` : undefined]
      .filter(Boolean).join(' ');
  }, [data, measure, from, scope, groupUnobserved]);

  const valuesAt = useCallback((hour: number): FlowValues & { diagram: ReturnType<typeof buildDiagram> } => {
    const diagram = buildDiagram(flows, measure, from, hour, perSide);
    const colour = new Map(diagram.nodes.flatMap(node => {
      if (!node.identity) return [];
      const raw = labelOf(node.identity).color;
      return [[node.id, fixed ? fixedColours.get(node.identity) ?? otherColour : raw ? toHex({ r: raw[0], g: raw[1], b: raw[2] }) : otherColour] as const];
    }));
    const selections = nodeSelections(flows, diagram.nodes);
    const byId = new Map(diagram.nodes.map(node => [node.id, node]));
    const nodes: TradeNode[] = diagram.nodes.map(node => {
      const label = node.identity ? labelOf(node.identity) : undefined;
      return { id: node.id, side: node.side, label: nodeName(node), color: colour.get(node.id) ?? otherColour,
        flag: label ? label.flag ?? null : undefined,
        countryFlag: label ? { flag: label.flag, flagReason: label.flagReason, ...(label.flagDetail !== undefined ? { flagDetail: label.flagDetail } : {}), flagPending: label.flagPending } : undefined,
        order: node.identity ? String(ranks[node.side].get(node.identity) ?? 0).padStart(7, '0') : OTHER, last: !node.identity,
        trade: { selection: selections.get(node.id)!, node, links: diagram.links, hour, from } };
    });
    const order = trade?.resourceOrder;
    const links: TradeLink[] = diagram.links.map(link => ({ id: link.id, source: link.source, target: link.target, value: link.value,
      color: options.colour === 'seller' ? colour.get(link.source) ?? otherColour : resourceColour.get(link.resource.id) ?? otherColour,
      order: String(order?.get(link.resource.id) ?? 0).padStart(6, '0'),
      trade: { selection: linkSelection(selections.get(link.source)!, selections.get(link.target)!, link.resource.id),
        link, source: byId.get(link.source)!, target: byId.get(link.target)!, hour, from } }));
    return { nodes, links, hour, message: messageAt(hour, diagram.links.length), diagram };
  }, [flows, measure, from, perSide, labelOf, fixed, fixedColours, otherColour, nodeName, ranks, trade, options.colour, resourceColour, messageAt]);
  const shown = useMemo(() => valuesAt(moment), [valuesAt, moment]);
  const live = useMemo(() => {
    if (!following || !track) return null;
    let told: number | null = null;
    const source = liveAt(runtime.clock, headAt, valuesAt);
    return { ...source, read: () => {
      const value = source.read();
      if (value.hour !== told) { told = value.hour ?? null; perfLog?.shown(widget.id, told); }
      return value;
    } };
  }, [following, track, runtime.clock, headAt, valuesAt, widget.id]);
  const texts = useMemo(() => following && track ? liveTexts(runtime.clock, () => headAt(runtime.clock.getSnapshot().position),
    at => new Map([['caption', measure === 'current' ? 'Per day' : `${measure === 'total' ? 'Delivered' : 'Average per trading day'} · ${dayLabel(from)} – ${dayLabel(at)}`],
      ['coverage', separated(coverageAt(at))]])) : null, [following, track, runtime.clock, headAt, measure, from, coverageAt]);

  const amount = useCallback((list: readonly Flow[], hour: number) => data ? formatAmount(measureAmount(list, measure, from, hour), data.decimals) : '', [data, measure, from]);
  const tooltip = useCallback((hit: FlowHit): TooltipContent | null => {
    if (!data || !trade) return null;
    const drawn = hit.kind === 'link' ? (hit.link as TradeLink).trade : (hit.node as TradeNode).trade;
    if (!drawn) return null;
    const note = coverageAt(drawn.hour) || undefined;
    const tooltipOptions = { measure, from: drawn.from, hour: drawn.hour, decimals: data.decimals, note, viewport,
      nodeName, labelAt: hoverLabels.labelAt, ask: askHover, countryAt: (identity: string, at: number) => identityCountryAt(trade, identity, at) };
    if (hit.kind === 'link') {
      const link = (hit.link as TradeLink).trade;
      return ribbonTooltip(link.link, link.source, link.target, tooltipOptions);
    }
    const node = (hit.node as TradeNode).trade;
    return nodeTooltip(node.node, node.links, { ...tooltipOptions, resourceOrder: trade.resourceOrder,
      resourceColour: id => options.colour === 'resource' ? resourceColour.get(id) : undefined });
  }, [data, trade, measure, hoverLabels.labelAt, askHover, nodeName, coverageAt, viewport, options.colour, resourceColour]);

  const headNow = useCallback(() => live ? live.hour() : moment, [live, moment]);
  const openCountry = useCallback((holder: string, hour: number) => {
    setSelection({ kind: 'country', holder });
    drillControl.show('contributions', hour);
  }, [widget, runtime.clock, following, headNow, playhead]);
  const onContextMenu = useCallback((hit: FlowHit | null, x: number, y: number) => {
    if (!track) return;
    const at = headNow();
    const items: MenuItem[] = [jumpItem(runtime, track.id, at)];
    if (hit?.kind === 'node') {
      const drawn = (hit.node as TradeNode).trade;
      const holder = drawn?.node.identity;
      if (holder) items.unshift({ label: `Imports and exports of ${hit.node.label}`, icon: 'flows', run: () => openCountry(holder, drawn.hour) });
    }
    const link = hit?.kind === 'link' ? (hit.link as TradeLink).trade?.link : null;
    if (link) {
      const sums = exactSums(link.flows, extent[0], extent[1]);
      if (sums.first != null) items.push({ ...jumpItem(runtime, track.id, sums.first), hint: 'First recorded in the time shown' });
      if (sums.last != null && sums.last !== sums.first) items.push({ ...jumpItem(runtime, track.id, sums.last), hint: 'Last recorded in the time shown' });
    }
    menu.open(x, y, items);
  }, [track, runtime, headNow, extent[0], extent[1], menu.open, openCountry]);

  const savedSelection = useMemo(() => track ? readSelection(options.tradeSelection, tradeContext(track)) : null, [options.tradeSelection, track]);
  const chosenSelection = pinnedView ? (options.tradeSelection ? savedSelection : ALL_CONTRIBUTORS) : selection;
  const contributors = useMemo(() => chosenSelection ? selectedContributors(flows, chosenSelection) : [], [flows, chosenSelection]);
  const known = useMemo(() => observedTradeSpans(data?.known ?? [], scope?.kind === 'countries' ? scope.unobserved : undefined), [data, scope]);
  const effectiveDrill = drill;
  const drillFollows = effectiveDrill.breakdown !== null && drillControl.following && !!track;
  const drillOpen = effectiveDrill.breakdown !== null;
  const drillTime = drillControl.region.action;
  const mainRegion = useRegion(widget);
  const drillRegion = useRegion(widget, drillTime);
  const mainRead = useRegionCoverage(runtime, mainRegion);
  const drillRead = useRegionCoverage(runtime, drillRegion);
  const drillHourNow = useRegionHour(runtime, drillRegion, active && drillOpen);
  const drillAt = !drillOpen || drillRead.outside ? null : drillFollows ? recordedHour(track, lookHourNow + axisOffset) : drillHourNow;

  const countryHolder = chosenSelection?.kind === 'country' ? chosenSelection.holder : null;
  const requested = all ? null : resolved ? [...resolved.tags] : [];
  const countryTags = trade && countryHolder ? countryTradeTags(trade, requested, countryHolder) : null;
  const countryReply = useTradeWindows(runtime.client, drillControl.gate, track, countryTags, extent[0], extent[1], !marketView && !!track && !!countryTags && effectiveDrill.breakdown != null,
    active && effectiveDrill.breakdown != null, track?.source);
  const countryData: TradeData | null = !countryHolder ? null : countryTags ? countryReply.data ? decodeTrade(countryReply.data) : null : trade ?? null;
  const countrySource = countryData ?? trade;
  const countryFact = countryHolder && drillAt != null && countrySource ? identityCountryAt(countrySource, countryHolder, drillAt) : null;
  const countryTitle = useCountryLabelsAt(runtime.client, track, countryFact ? [{ tag: countryFact.tag, hour: countryFact.hour }] : [], active);
  const drillTitle = !countryHolder ? 'Trade contributors'
    : countryFact ? `${countryTitle.labelAt(countryFact.hour)(countryFact.tag).name}: imports and exports` : 'Imports and exports';
  const drillLive = useMemo<LiveHour | null>(() => {
    if (!drillFollows || !trade) return null;
    const hour = () => headAt(runtime.clock.getSnapshot().position);
    const base = liveAt(runtime.clock, headAt, at => at);
    const drillTexts = tradeTexts(trade, measure, from, known);
    const ownTexts = countryHolder && countryData ? countryTexts(countryData, countryHolder, resourceIds, measure, from) : null;
    return { ...base, texts: liveTexts(runtime.clock, hour, at => ownTexts ? new Map([...drillTexts(at), ...ownTexts(at)]) : drillTexts(at)) };
  }, [drillFollows, trade, runtime.clock, headAt, measure, from, known, countryHolder, countryData, resourceIds]);

  const headerTag = pinnedView && widget.countries.kind === 'tags' && widget.countries.tags.length === 1
    ? contributionCountryTag(identities, widget.countries.tags[0], chosenSelection, drillAt) : null;
  const header = useCountryLabelsAt(runtime.client, track, countrySlot && headerTag && drillAt !== null ? [{ tag: headerTag, hour: drillAt }] : [], active);
  const pinnedHeader = !marketView && pinnedView && countrySlot && widget.countries.kind === 'tags' && widget.countries.tags.length === 1
    ? createPortal(<DatedCountryChip hour={drillAt} label={headerTag && drillAt !== null ? header.labelAt(drillAt)(headerTag) : null}
      reason={headerTag ? null : 'Selected country unavailable.'} live={drillLive && !drillRead.outside ? { read: drillLive.hour, subscribe: drillLive.subscribe } : undefined} />, countrySlot) : null;
  const openDrill = useCallback((hit: FlowHit, hour: number) => {
    if (!track) return;
    const drawn = hit.kind === 'node' ? (hit.node as TradeNode).trade : (hit.link as TradeLink).trade;
    if (!drawn) return;
    setSelection(drawn.selection);
    drillControl.show('contributions', hour);
  }, [track, widget, runtime.clock, following, headNow, playhead]);
  const openLabel = useCallback((node: FlowNode, hour: number) => {
    const drawn = (node as TradeNode).trade;
    if (drawn?.node.identity) openCountry(drawn.node.identity, hour);
    else openDrill({ kind: 'node', node }, hour);
  }, [openCountry, openDrill]);

  const mainMembers = useCountriesSelectionAt(widget, runtime, active && marketView);
  const drillMembers = useCountriesSelectionAt(widget, runtime, active && marketView && drillOpen, drillTime);
  const mainMarket = marketSelectionAt(mainMembers, marketView, all);
  const drillMarket = marketSelectionAt(drillMembers, marketView && drillOpen, all);
  const marketTags = mainMarket.tags, marketReason = mainMarket.reason;
  const marketReady = !!track && marketView && mainMarket.ready;
  const market = useHeadProduct(runtime, mainMembers.gate, 'market-lendlease', marketReady ? { countries: marketTags.map(countryFromKey) } : null,
    { active: active && marketView, validate: validateMarketReply });
  const pickedMarketHead = useHeadProduct(runtime, drillMembers.gate, 'market-lendlease',
    !!track && marketView && drillOpen && drillMarket.ready ? { countries: drillMarket.tags.map(countryFromKey) } : null,
    { active: active && marketView && drillOpen, validate: validateMarketReply });

  useEffect(() => {
    const warnings = [...(marketView ? market.data?.warnings ?? [] : data?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, market.data, marketView, groups.data, onDiagnostics]);

  const description = useMemo(() => {
    if (!data || !shown.diagram.links.length) return shown.message ?? '';
    const nodeById = new Map(shown.diagram.nodes.map(node => [node.id, node]));
    const sorted = [...shown.diagram.links].sort((a, b) => b.value - a.value);
    const listed = sorted.slice(0, DESCRIBED).map(link => `${nodeName(nodeById.get(link.source)!)} to ${nodeName(nodeById.get(link.target)!)}, ${resourceName(link.resource)} ${amount(link.flows, moment)}`);
    return `${measureNote(measure)}, ${measure === 'current' ? dayLabel(moment) : `${dayLabel(from)} to ${dayLabel(moment)}`}: ${listed.join('; ')}${sorted.length > DESCRIBED ? `; and ${sorted.length - DESCRIBED} more` : ''}.`;
  }, [data, shown, nodeName, amount, measure, moment, from]);

  if (!track) return <>{pinnedHeader}<WidgetState kind="empty" icon="flows" message="Load a campaign to see who trades with whom." /></>;
  if (marketView && market.outside && !drillOpen) return <WidgetState kind="outside" read={market} />;
  if (!marketView && (pinnedView ? drillRead.outside : mainRead.outside && !drillOpen)) return <>{pinnedHeader}<WidgetState kind="outside" read={pinnedView ? drillRead : mainRead} /></>;
  const groupFailure = (widget.countries.kind === 'group' || marketView && !all) && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  const pickDay = (hour: number) => dispatchDrill(drill.breakdown ? { type: 'pick', hour } : { type: 'open', breakdown: 'contributions', hour, follow: false });
  const drillMenu = (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]);
  const frame = (main: ReactNode) => <div className="trade-flows" ref={anchor}>{groupFailure}
    {pinnedHeader}
    <DrillHost drill={drillControl} runtime={runtime} subject={effectiveDrill.breakdown != null} mode={pinnedView || options.drill !== 'popout' ? 'inline' : 'popout'} anchor={anchor}
      title={drillTitle} day={drillRead.outside ? '' : drillLive ? <LiveText live={drillLive.texts} at="caption-day" /> : drillAt == null ? '' : dayLabel(drillAt)} canFollow
      returnFocus=".flow-diagram canvas" onClose={() => {
        dispatchDrill({ type: 'close' });
        if (pinnedView) setOption?.('show', 'main');
      }}
      pinRefused={drillControl.pinRefused} onPin={chosenSelection && drillControl.time() ? () => {
        const time = drillControl.time();
        if (time) runtime.addWidgetBelow(widget.id, pinContributions(widget, track, chosenSelection, time, extent, countryFact?.tag ?? null));
      } : undefined} main={main}>
      {drillRead.outside ? <WidgetState kind="outside" read={drillRead} />
        : countryHolder ? countryReply.error && countryTags ? <WidgetState kind="error" failure={countryReply.error} onRetry={countryReply.retry} />
        : !countryData ? <WidgetState kind="loading" label="Loading this country's trade…" />
        : <TradeCountryView holder={countryHolder} trade={countryData} resources={resourceIds} track={track} runtime={runtime} measure={measure} from={from}
          hour={drillAt} live={drillLive} extent={extent} colours={resourceColour} tokens={tokens} active={active && effectiveDrill.breakdown != null}
          onPick={pickDay} onContextMenu={drillMenu} />
      : !trade ? error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : <WidgetState kind="loading" label="Loading trade…" />
      : chosenSelection && <TradeDrill flows={contributors} trade={trade} track={track} runtime={runtime} measure={measure} from={from}
        hour={drillAt} live={drillLive} extent={extent} known={known} colours={resourceColour} tokens={tokens}
        identityNote={scope?.kind === 'countries' ? scope.identityNote : undefined} active={active && effectiveDrill.breakdown != null}
        onPick={pickDay} onContextMenu={drillMenu} />}
    </DrillHost>
    {menu.menu}
  </div>;
  const drillShown = effectiveDrill.breakdown != null && !marketView;
  if (groupFailure && !groups.data) return drillShown ? frame(groupFailure) : <>{pinnedHeader}{groupFailure}</>;
  if (marketView) {
    const pickedMarket = pinnedView ? savedSelection?.kind === 'market' ? savedSelection.key : null : marketSelection;
    const loadingMarket = <WidgetState kind="loading" label="Loading market and lend-lease…" />;
    const parent = market.outside ? <WidgetState kind="outside" read={market} />
      : marketReason ? <WidgetState kind="unavailable" message={marketReason} />
      : market.error ? <WidgetState kind="error" failure={market.error} onRetry={market.retry} />
      : !market.data || market.at === null || (widget.countries.kind === 'group' && !groups.data) ? loadingMarket : null;
    if (parent && !drillOpen) return <>{groupFailure}{parent}</>;
    if (pinnedView && !pickedMarket) return <>{groupFailure}<WidgetState kind="unavailable" message="This market selection is unavailable. Select it again." /></>;
    const deliveriesAt = (hour: number, follows: boolean, tags: readonly string[], at: Gate) => <MarketOccurrences runtime={runtime} track={track} kinds={DELIVERY_KINDS}
      tags={all ? null : tags} span={at.span(track.branch.firstHour, track.branch.lastHour)} cut={follows ? { follow: true, offset: track.axisOffset } : { follow: false, hour }} active={active}
      label="Market deliveries" density="compact" months={false} animate={false} />;
    const marketData = market.data && market.at !== null ? { ...market.data, hour: market.at } : null;
    const main = parent ?? (marketData && <div ref={market.root} style={{ display: 'contents' }}><TradeMarketView data={marketData} values={market.live.read()}
      deliveries={deliveriesAt(marketData.hour, following, marketTags, mainMembers.gate)} onPick={key => {
        setMarketSelection(key); drillControl.show('contributions', marketData.hour);
      }} /></div>);
    const drillHead = pickedMarketHead;
    const drillData = drillHead.data && drillHead.at !== null ? { ...drillHead.data, hour: drillHead.at } : null;
    const drillBody = drillHead.outside || drillMarket.outside ? <WidgetState kind="outside" read={drillHead} />
      : drillMarket.reason ? <WidgetState kind="unavailable" message={drillMarket.reason} />
      : drillHead.error ? <WidgetState kind="error" failure={drillHead.error} onRetry={drillHead.retry} />
      : !drillData ? loadingMarket
      : <div ref={drillHead.root} style={{ display: 'contents' }}><TradeMarketView data={drillData} selected={pickedMarket} values={drillHead.live.read()}
        deliveries={deliveriesAt(drillData.hour, drillFollows, drillMarket.tags, drillMembers.gate)} /></div>;
    return <div className="trade-flows" ref={anchor}>{groupFailure}
      <DrillHost drill={drillControl} runtime={runtime} subject={drillOpen} mode={!pinnedView && options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
        title="Market & lend-lease" day={drillData ? dayLabel(drillData.hour) : ''} canFollow
        returnFocus=".market-lendlease summary" onClose={() => { dispatchDrill({ type: 'close' }); if (pinnedView) setOption?.('show', 'main'); }}
        pinRefused={drillControl.pinRefused} onPin={!pickedMarket || !drillData || !drillControl.time() ? undefined : () => {
          const time = drillControl.time();
          if (!time) return;
          const pinned = pinContributions(widget, track, { kind: 'market', key: pickedMarket }, time, extent);
          runtime.addWidgetBelow(widget.id, { ...pinned, title: 'Market & lend-lease', options: { ...pinned.options, view: 'market' } });
        }} main={main}>
        {drillBody}
      </DrillHost>
    </div>;
  }
  const atMoment = worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, moment, lastHour, identities);
  const selectionReason = selectionMessage(resolved, atMoment, measure, from, moment, groupUnobserved);
  if (selectionReason && !pinnedView && !drill.breakdown) return <>{groupFailure}<WidgetState kind="empty" message={selectionReason} /></>;
  if (error) return drillShown ? frame(<WidgetState kind="error" failure={error} onRetry={retry} />) : <>{groupFailure}{pinnedHeader}<WidgetState kind="error" failure={error} onRetry={retry} /></>;
  const none = measure === 'current' && resolved ? noMembersAt(widget.countries, resolved, groups.data, moment, lastHour) : null;
  if (none && !pinnedView && !drill.breakdown) return <>{groupFailure}{pinnedHeader}<NoMembersState reason={none.reason} icon="flows" /></>;
  if (!data || !trade || (widget.countries.kind === 'group' && !groups.data)) return drillShown ? frame(<WidgetState kind="loading" label="Loading trade…" />)
    : <>{groupFailure}{pinnedHeader}<WidgetState kind="loading" label="Loading trade…" /></>;
  if (!data.known.length) return <>{groupFailure}{pinnedHeader}<WidgetState kind="unavailable" message="Trade was not recorded." /></>;
  if (resourceIds && !resourceIds.size) return <>{groupFailure}{pinnedHeader}<WidgetState kind="empty" message="None of the chosen resources is in this campaign. Choose others in the widget settings." /></>;
  if (pinnedView && options.tradeSelection && (!savedSelection || savedSelection.kind === 'market')) return <>{groupFailure}{pinnedHeader}<WidgetState kind="unavailable" message="This trade selection is unavailable. Select trades again." /></>;

  const caption = texts ? <LiveText live={texts} at="caption" /> : measure === 'current' ? 'Per day'
    : `${measure === 'total' ? 'Delivered' : 'Average per trading day'} · ${dayLabel(from)} – ${dayLabel(moment)}`;
  const coverage = texts ? <LiveText live={texts} at="coverage" /> : separated(coverageAt(moment));
  const main = mainRead.outside ? <WidgetState kind="outside" read={mainRead} /> : <div className="trade-main">
    <p className="trade-caption">{caption}{coverage}</p>
    <Legend always label="Resources" items={resourceLegend} windowed={shownResources.length > tokens.categories.length} />
    {options.colour === 'seller' ? <p className="chart-line-key">Ribbons in each seller’s map colour</p> : null}
    <div className="trade-inventory-action"><button type="button" className="button small quiet" onClick={() => {
      setSelection(ALL_CONTRIBUTORS);
      drillControl.show('contributions', headNow());
    }}>All trades</button></div>
    <FlowDiagram label={`Trade flows: sellers to buyers${all ? ', all countries' : ''}`} nodes={shown.nodes} links={shown.links} live={live ?? undefined}
      stretch={measure === 'current' ? knownSpanAt(data.known, moment) : 'all'} scale={scale} slots={perSide + 1} hour={moment} onPick={openDrill}
      hoursPerSecond={motionOf(runtime.clock.getSnapshot())} message={shown.message} tooltip={tooltip} onContextMenu={onContextMenu} description={description} onCapacity={setCapacity}
      onLabelPick={openLabel}
      active={active && !(effectiveDrill.breakdown && (pinnedView || options.drill !== 'popout'))} />
  </div>;
  return frame(main);
}
