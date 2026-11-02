import { nameReadFrom, nameReadMetadata } from '../../dashboard/nameReads.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { dayLabel } from '../../charts/draw';
import FlowDiagram from '../../charts/flow/FlowDiagram';
import type { FlowHit, FlowLink, FlowNode } from '../../charts/flow/FlowDiagram';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { usNumber } from '../../charts/numberFormat';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount } from '../../charts/valueAxis';
import { MAX_NAME_HOURS, isFailedFact } from '../../dashboard/countryFacts.ts';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { useContentRevision } from '../../dashboard/contentRevision';
import { flagReadMetadata, useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryFlagsRead } from '../../dashboard/countryFlags';
import { lookHour } from '../../dashboard/countryLooks';
import { nameKey, useCountryFactsRead } from '../../dashboard/countryNamesAt';
import type { NameAt } from '../../dashboard/countryNamesAt';
import { STYLES } from '../../dashboard/useCountriesAt';
import { sampledLooks } from '../../dashboard/looks';
import type { Look } from '../../dashboard/looks';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING, noMembersAt, worldFilter } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { readAt } from '../../dashboard/reads';
import { mix, toHex } from '../../shell/appearance/color';
import { mergePreparedTrade, buildDiagram, campaignScale, coverageNote, sideRanks, decodeTrade, exactSums, formatDays, heldScale, knownHours, emptyMessage, knownSpanAt, unrecordedMessage, measureAmount, measureNote, OTHER, resourceColours, selectionMessage, identityCountryAt,
  resourceName, selectFlows, validateTradeFlowsReply } from './tradeData.ts';
import type { Flow, HeldScale, Measure, SideNode, SideLink } from './tradeData';
import TradeDrill from './TradeDrillView';
import TradeCountryView, { resourceLegendItems } from './TradeCountryView';
import { countryTradeTags } from './tradeCountry';
import { useParityRows } from '../../dashboard/parityRows';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import type { PreparedTradeReply } from './tradeData';
import { contributionCountryTag, contributionExtent, contributionHour, linkSelection, nodeSelections, observedTradeSpans, openContributions, pinContributions, readSelection, selectedContributors, tradeContext } from './tradeDrill';
import type { TradeSelection } from './tradeDrill';
import { heldTradeColours, tradeColourPlan, tradeCountryColours, tradeFixedColourKey } from './tradeColours';
import { OPTIONS } from './definition';
import { tradeFlowsRequest, useTradeFlows } from './useTradeFlows';
import { nodeTooltip, ribbonTooltip } from './tradeTooltip';
import './tradeFlows.css';
import TradeMarketView from './TradeMarketView';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];
/** Ribbons the screen-reader description lists, largest first. */
const DESCRIBED = 24;

type Label = CountryFlagLook & { name: string; color: readonly [number, number, number] | null };
type TradeNode = FlowNode & { trade: { selection: TradeSelection; node: SideNode; links: readonly SideLink[]; hour: number; from: number } };
type TradeLink = FlowLink & { trade: { selection: TradeSelection; link: SideLink; source: SideNode; target: SideNode; hour: number; from: number } };
const ALL_CONTRIBUTORS: TradeSelection = { kind: 'all' };

/** Who sells what to whom: a Sankey of sellers to buyers, one
 * ribbon per seller, buyer and resource, sized by the chosen measure. */
export default function TradeFlowsWidget({ widget, runtime, active, countrySlot, onDiagnostics, setOption }: WidgetProps) {
  const [drill, dispatchDrill] = useReducer(reduceDrill, CLOSED);
  const [selection, setSelection] = useState<TradeSelection | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  const options = resolveOptions(OPTIONS, widget.options);
  const marketView = options.view === 'market';
  const pinnedView = options.show === 'contributions';
  const [marketSelection, setMarketSelection] = useState<string | null>(null);
  const marketTrack = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const playbackHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || (!!drill.breakdown && drill.follow)));
  const marketDrillHour = marketView && marketTrack && drill.breakdown !== null ? contributionHour(drill, marketTrack, playbackHour) ?? undefined : undefined;
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const measure = (options.measure as Measure) ?? 'current';
  const asked = Number(options.perSide ?? 12);
  // As many countries a side as the diagram fits at this size (the flow kit says),
  // one slot kept for the rest summed.
  const [capacity, setCapacity] = useState<number | null>(null);
  const perSide = capacity == null ? asked : Math.max(1, Math.min(asked, capacity - 1));
  const incoming = useTradeFlows(widget, runtime, active, marketDrillHour);
  const { track, all, resolved, groups, data, scope, retry } = incoming;
  const error = useHeldFailure(incoming.error, data !== undefined, retry);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const savedExtent = track && pinnedView ? contributionExtent(options.tradeExtent, track.branch) : null;
  const extent: [number, number] = savedExtent ?? (track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1]);
  // While playback's newest hour is read, the last hour read stays shown with its date.
  const shownAt = (hour: number, held: number | undefined, exact: ((hour: number) => boolean) | undefined) => exact && !exact(hour)
    && held !== undefined && held >= extent[0] && held <= extent[1] && exact(held) ? held : hour;
  const requestedHour = Math.floor(playhead ?? extent[1]);
  const moment = shownAt(requestedHour, incoming.reachedHour, incoming.hasExactHour);
  const from = extent[0];
  useParityRows(anchor, () => ({ kind: 'chart', label: 'Trade flow values', complete: incoming.parityComplete,
    total: data?.flows.length ?? 0, rows: data ? [{ branchId: data.branchId, decimals: data.decimals, resources: data.resources, countries: data.countries,
      countryIdentities: data.countryIdentities, reads: data.reads, known: data.known, partial: data.partial, flows: data.flows, warnings: data.warnings }] : [] }),
    { prepare: incoming.parityPrepare, ready: () => incoming.parityComplete, error: () => error ?? null });
  const menu = useContextMenu('Trade actions');

  const trade = useMemo(() => data && decodeTrade(data), [data]);
  const identities = useMemo(() => data ? countryIdentityIndex(data.countryIdentities.map(country => ({ tag: country.tag, identities: country.runs }))) : null, [data]);
  const chosen = contentSet(options.resources);
  const chosenKey = JSON.stringify(chosen);
  const resourceIds = useMemo(() => {
    if (!trade || !chosen.length) return null;
    return new Set(trade.reply.resources.filter(item => chosen.includes(item.token)).map(item => item.id));
  }, [trade, chosenKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const flows = useMemo(() => trade && scope ? selectFlows(trade, scope, resourceIds) : [], [trade, scope, resourceIds]);
  const diagram = useMemo(() => buildDiagram(flows, measure, from, moment, perSide), [flows, measure, from, moment, perSide]);
  const sized = options.scale !== 'moment';
  const readScale = useMemo(() => !sized || !incoming.scaleData || !scope ? undefined
    : campaignScale(selectFlows(decodeTrade(incoming.scaleData), scope, resourceIds), measure, from, extent[1]),
  [incoming.scaleData, scope, resourceIds, measure, from, extent[1], sized]); // eslint-disable-line react-hooks/exhaustive-deps
  const fallback = defaultCountryTag(track);
  const scaleScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries,
    widget.countries.kind === 'selected' ? fallback : null, options.between, chosenKey, measure]);
  const scaleHeld = useRef({ client: runtime.client, held: undefined as HeldScale | undefined });
  if (scaleHeld.current.client !== runtime.client) scaleHeld.current = { client: runtime.client, held: undefined };
  scaleHeld.current.held = heldScale(scaleHeld.current.held, scaleScope, readScale);
  // Undefined until this scope's campaign scale is read: never the moment's fit instead.
  const scale = sized ? scaleHeld.current.held.scale : null;
  const scaleFailure = useHeldFailure(sized ? incoming.scaleError : undefined, scale !== undefined, incoming.retryScale);
  // Each country's fixed place for ribbons inside the other side's nodes.
  const ranks = useMemo(() => sideRanks(flows, extent[0], extent[1]), [flows, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps

  const read = data ? readAt(data.reads, moment) : null;
  const clock = runtime.clock.getSnapshot();
  // Sample within each native holder/alias segment. Two historical holders sharing
  // a tag retain separate dated facts, including in the total/average diagram.
  const lookFacts = useMemo(() => {
    const facts = new Map<string, NameAt & { stretch: number }>();
    if (trade && track) for (const node of diagram.nodes) {
      const fact = node.identity ? identityCountryAt(trade, node.identity, moment) : null;
      const sample = fact && lookHour(track.branch, fact.hour, fact.firstHour);
      if (fact && sample) facts.set(fact.identity, { tag: fact.tag, hour: sample.hour, stretch: sample.stretch });
    }
    return facts;
  }, [trade, track, diagram, moment]);
  const looksWanted = useMemo(() => [...lookFacts.values()], [lookFacts]);
  const lookHours = Math.max(MAX_NAME_HOURS, new Set(looksWanted.map(item => item.hour)).size);
  const fixed = tokens.countryColours === 'fixed';
  const colourPlan = useMemo(() => tradeColourPlan(trade ?? null, track?.branch, moment, fixed), [trade, track?.branch, moment, fixed]);
  const styles = useCountryFactsRead(runtime.client, track, fixed ? colourPlan.wanted : looksWanted, active, STYLES, fixed ? colourPlan.maxHours : lookHours);
  const flags = useCountryFlagsRead(runtime.client, track, looksWanted, active);
  const contentRevision = useContentRevision(runtime.client);
  const heldKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, contentRevision]);
  const held = useRef({ key: heldKey, client: runtime.client, looks: new Map<string, Look>(), segments: new Map<string, number>() });
  if (held.current.key !== heldKey || held.current.client !== runtime.client) held.current = {
    key: heldKey, client: runtime.client, looks: new Map(), segments: new Map(),
  };
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
  if (heldColours.current.key !== colourContext || heldColours.current.client !== runtime.client) heldColours.current = {
    key: colourContext, client: runtime.client, colours: new Map(), selection: colourPlan.selection,
  };
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
  const hoverLabels = useCountryLabelsAt(runtime.client, track, hoverWanted, active,
    Math.max(MAX_NAME_HOURS, new Set(hoverWanted.map(item => item.hour)).size));
  useEffect(() => {
    dispatchDrill({ type: 'close' }); setSelection(null); setMarketSelection(null); hoverKey.current = ''; setHoverWanted([]);
  }, [track?.source.id, track?.source.revision, track?.branch.id, options.view]);
  useEffect(() => { if (!active) { hoverKey.current = ''; setHoverWanted([]); } }, [active]);

  useEffect(() => {
    const warnings = [...(marketView ? incoming.market?.data?.warnings ?? [] : data?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, incoming.market?.data, marketView, groups.data, onDiagnostics]);

  // What the diagram draws: nodes named and coloured, ribbons by resource or seller.
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
  const { flowNodes, flowLinks } = useMemo(() => {
    const colour = new Map(diagram.nodes.flatMap(node => {
      if (!node.identity) return [];
      const raw = labelOf(node.identity).color;
      return [[node.id, fixed ? fixedColours.get(node.identity) ?? otherColour
        : raw ? toHex({ r: raw[0], g: raw[1], b: raw[2] }) : otherColour] as const];
    }));
    const selections = nodeSelections(flows, diagram.nodes);
    const nodes = new Map(diagram.nodes.map(node => [node.id, node]));
    const flowNodes: TradeNode[] = diagram.nodes.map(node => {
      const label = node.identity ? labelOf(node.identity) : undefined;
      return { id: node.id, side: node.side, label: nodeName(node), color: colour.get(node.id) ?? otherColour,
        flag: label ? label.flag ?? null : undefined,
        countryFlag: label ? { flag: label.flag, flagReason: label.flagReason, ...(label.flagDetail !== undefined ? { flagDetail: label.flagDetail } : {}), flagPending: label.flagPending } : undefined,
        order: node.identity ? String(ranks[node.side].get(node.identity) ?? 0).padStart(7, '0') : OTHER, last: !node.identity,
        trade: { selection: selections.get(node.id)!, node, links: diagram.links, hour: moment, from } };
    });
    const order = trade?.resourceOrder;
    const flowLinks: TradeLink[] = diagram.links.map(link => ({ id: link.id, source: link.source, target: link.target, value: link.value,
      color: options.colour === 'seller' ? colour.get(link.source) ?? otherColour : resourceColour.get(link.resource.id) ?? otherColour,
      order: String(order?.get(link.resource.id) ?? 0).padStart(6, '0'),
      trade: { selection: linkSelection(selections.get(link.source)!, selections.get(link.target)!, link.resource.id),
        link, source: nodes.get(link.source)!, target: nodes.get(link.target)!, hour: moment, from } }));
    return { flowNodes, flowLinks };
  }, [diagram, flows, labelOf, nodeName, fixed, fixedColours, otherColour, resourceColour, options.colour, trade, ranks, moment, from]);

  const spanIndex = data ? knownSpanAt(data.known, moment) : -1;
  // The countries at the moment shown, by the kit's whole-world rule (`worldFilter`):
  // its reason says when a group's members there are unknown or none. Totals over
  // time of a group counted at each hour count every member while it was one.
  const lastHour = track?.branch.lastHour ?? 0;
  const atMoment = worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, moment, lastHour, identities);
  const momentReason = atMoment.reason && (measure === 'current' || !resolved?.membership) ? atMoment.reason : null;
  const groupUnobserved = widget.countries.kind === 'group' && widget.countries.membership === 'hour' ? resolved?.unobserved ?? null : null;
  // Nothing recorded is said as such for every measure, never as no trade; deals a read
  // could not attribute leave the hour known only in part, which is said too.
  const unrecorded = data ? unrecordedMessage(data.known, measure, from, moment) : null;
  const message = !data ? null : unrecorded
    ?? momentReason ?? (diagram.links.length ? null
      // Members at hours still being prepared are not known yet: no trade is not proof of none.
      : !atMoment.known || resolved && !resolved.known(measure === 'current' ? moment : from, moment) ? MEMBERSHIP_PREPARING
        : scope?.kind === 'countries' && scope.identityNote ? scope.identityNote : emptyMessage(data, measure, from, moment, groupUnobserved));
  // Totals and averages cover recorded hours only: unrecorded deals, and hours a group
  // counted at each hour has no recorded membership, are said, never counted as none.
  const groupHours = groupUnobserved ? knownHours(groupUnobserved, from, moment) : 0;
  const coverage = data ? [coverageNote(data, measure, from, moment),
    scope?.kind === 'countries' ? scope.identityNote : undefined,
    measure !== 'current' && groupHours >= 1 ? `${formatDays(groupHours)} of the group’s membership in this time were not recorded; they are not counted.` : undefined]
    .filter(Boolean).join(' ') || undefined : undefined;
  const note = coverage;
  const nodeById = useMemo(() => new Map(diagram.nodes.map(node => [node.id, node])), [diagram]);

  const amount = useCallback((list: readonly Flow[]) => data ? formatAmount(measureAmount(list, measure, from, moment), data.decimals) : '', [data, measure, from, moment]);
  const tooltip = useCallback((hit: FlowHit): TooltipContent | null => {
    if (!data || !trade) return null;
    const drawn = hit.kind === 'link' ? (hit.link as TradeLink).trade : (hit.node as TradeNode).trade;
    if (!drawn) return null;
    const tooltipOptions = { measure, from: drawn.from, hour: drawn.hour, decimals: data.decimals, note, viewport,
      nodeName, labelAt: hoverLabels.labelAt, ask: askHover, countryAt: (identity: string, at: number) => identityCountryAt(trade, identity, at) };
    if (hit.kind === 'link') {
      const drawn = (hit.link as TradeLink).trade;
      return ribbonTooltip(drawn.link, drawn.source, drawn.target, tooltipOptions);
    }
    const node = (hit.node as TradeNode).trade;
    return nodeTooltip(node.node, node.links, { ...tooltipOptions, resourceOrder: trade.resourceOrder,
      resourceColour: id => options.colour === 'resource' ? resourceColour.get(id) : undefined });
  }, [data, trade, measure, hoverLabels.labelAt, askHover, nodeName, note, viewport, options.colour, resourceColour]);

  const openCountry = useCallback((holder: string, hour: number) => {
    setSelection({ kind: 'country', holder });
    dispatchDrill(openContributions(widget, hour, clock.playing, playhead));
  }, [widget, clock.playing, playhead]);
  const onContextMenu = useCallback((hit: FlowHit | null, x: number, y: number) => {
    if (!track) return;
    // The moment shown, as read for the current amount; a ribbon adds when it was
    // first and last recorded in the time shown.
    const items: MenuItem[] = [jumpItem(runtime, track.id, measure === 'current' ? read ?? moment : moment)];
    // The country's own view, also from the keyboard (the context-menu key on its node).
    if (hit?.kind === 'node') {
      const drawn = (hit.node as TradeNode).trade;
      const holder = drawn?.node.identity;
      if (holder) items.unshift({ label: `Imports and exports of ${hit.node.label}`, icon: 'flows', run: () => openCountry(holder, drawn.hour) });
    }
    const link = hit?.kind === 'link' ? (hit.link as TradeLink).trade?.link : null;
    if (link && !link.flows.some(flow => flow.prepared?.cumulativePending)) {
      const sums = exactSums(link.flows, extent[0], extent[1]);
      if (sums.first != null) items.push({ ...jumpItem(runtime, track.id, sums.first), hint: 'First recorded in the time shown' });
      if (sums.last != null && sums.last !== sums.first) items.push({ ...jumpItem(runtime, track.id, sums.last), hint: 'Last recorded in the time shown' });
    }
    menu.open(x, y, items);
  }, [track, runtime, measure, read, moment, extent[0], extent[1], menu.open, openCountry]); // eslint-disable-line react-hooks/exhaustive-deps

  const savedSelection = useMemo(() => track ? readSelection(options.tradeSelection, tradeContext(track)) : null,
    [options.tradeSelection, track]);
  const chosenSelection = pinnedView ? (options.tradeSelection ? savedSelection : ALL_CONTRIBUTORS) : selection;
  const contributors = useMemo(() => chosenSelection ? selectedContributors(flows, chosenSelection) : [], [flows, chosenSelection]);
  const known = useMemo(() => observedTradeSpans(data?.known ?? [], scope?.kind === 'countries' ? scope.unobserved : undefined), [data, scope]);
  const effectiveDrill = drill.breakdown ? drill : pinnedView
    ? { breakdown: 'contributions', follow: widget.time.mode === 'follow', hour: moment } : CLOSED;
  const drillAt = track ? contributionHour(effectiveDrill, track, playbackHour) : null;
  useEffect(() => { incoming.onReachedHour?.(effectiveDrill.breakdown !== null ? drillAt : null); }, [drillAt, effectiveDrill.breakdown, incoming.onReachedHour]);
  const headerTag = pinnedView && widget.countries.kind === 'tags' && widget.countries.tags.length === 1
    ? contributionCountryTag(identities, widget.countries.tags[0], chosenSelection, drillAt) : null;
  const header = useCountryLabelsAt(runtime.client, track, countrySlot && headerTag && drillAt !== null ? [{ tag: headerTag, hour: drillAt }] : [], active);
  const pinnedHeader = !marketView && pinnedView && countrySlot && widget.countries.kind === 'tags' && widget.countries.tags.length === 1
    ? createPortal(<DatedCountryChip hour={drillAt} label={headerTag && drillAt !== null ? header.labelAt(drillAt)(headerTag) : null}
      reason={headerTag ? null : 'Selected country unavailable.'} />, countrySlot) : null;
  const openDrill = useCallback((hit: FlowHit, hour: number) => {
    if (!track) return;
    const drawn = hit.kind === 'node' ? (hit.node as TradeNode).trade : (hit.link as TradeLink).trade;
    if (!drawn) return;
    setSelection(drawn.selection);
    dispatchDrill(openContributions(widget, hour, clock.playing, playhead));
  }, [track, widget, clock.playing, playhead]);
  // A country's flag or name opens its whole trade; the summed rest opens its contributors.
  const openLabel = useCallback((node: FlowNode, hour: number) => {
    const drawn = (node as TradeNode).trade;
    if (drawn?.node.identity) openCountry(drawn.node.identity, hour);
    else openDrill({ kind: 'node', node }, hour);
  }, [openCountry, openDrill]);

  // One country's view needs its whole trade: the reply has it for every country or
  // when one of its tags was asked for; otherwise that country's own reply is read.
  const countryHolder = chosenSelection?.kind === 'country' ? chosenSelection.holder : null;
  const requested = all ? null : resolved ? [...resolved.tags] : [];
  const countryTags = trade && countryHolder ? runtime.client.preparedQuery
    ? [...new Set((trade.countryFacts.get(countryHolder) ?? []).map(fact => fact.tag))] : countryTradeTags(trade, requested, countryHolder) : null;
  const countryKey = countryTags ? JSON.stringify(countryTags) : null;
  const countryPage = incoming.page ? { ...incoming.page, identities: undefined, membership: undefined, unobserved: [], between: false,
    hours: [extent[1], ...(drillAt === null ? [] : [drillAt])] } : undefined;
  const countryReply = useFollowedProduct<PreparedTradeReply>(runtime.client, !marketView && track && countryTags && effectiveDrill.breakdown != null ? tradeFlowsRequest(track, countryTags, countryPage) : null,
    ['from', 'to', 'width', 'hours'], active && effectiveDrill.breakdown != null, value => validateTradeFlowsReply(value) as PreparedTradeReply, track?.source);
  const countryAnswered = countryReply.data && countryReply.moment ? JSON.parse(countryReply.moment) as [number, number, number, number[] | null] : undefined;
  const countryPoint = useProduct<PreparedTradeReply>(runtime.client, runtime.client.preparedQuery && !marketView && track && countryTags && incoming.pointHour !== null && incoming.pointHour !== undefined
    ? tradeFlowsRequest(track, countryTags, { ...countryPage, from: incoming.pointHour, to: incoming.pointHour, width: 1, hours: [incoming.pointHour] }) : null,
    active && effectiveDrill.breakdown != null, value => validateTradeFlowsReply(value) as PreparedTradeReply, track?.source);
  const countryRetained = useMemo(() => new NumericReply<PreparedTradeReply>(), [runtime.client]);
  const countryNumbers = countryRetained.update(JSON.stringify([track?.source.id, track?.source.revision, countryHolder, from, extent[1], measure]), countryReply.data,
    data => ({ ...data, resources: data.resources.map(resource => ({ ...resource, name: null, icon: null })), warnings: [] }));
  const countryData = useMemo(() => {
    const base = countryReply.data ?? countryNumbers;
    return !countryHolder ? null : countryKey ? base ? decodeTrade(countryPoint.data ? mergePreparedTrade(base, countryPoint.data) : base) : null : trade;
  }, [countryHolder, countryKey, countryReply.data, countryNumbers, countryPoint.data, trade]);
  const navigateFlows = incoming.navigateFlows;
  const countryPrepared = { ...incoming, navigateFlows: navigateFlows && countryTags ? (flows: readonly string[]) => navigateFlows(flows, countryTags) : undefined,
    hasExactHour: (hour: number) => !runtime.client.preparedQuery ||
    ((countryData?.reply as PreparedTradeReply | undefined)?.exactHours ?? []).includes(Math.floor(hour)) };
  const countryShown = drillAt === null ? null : shownAt(drillAt, countryAnswered?.[3]?.[1], countryPrepared.hasExactHour);
  const drillShown = drillAt === null ? null : shownAt(drillAt, incoming.reachedDetail, incoming.hasExactHour);
  const countrySource = countryData ?? trade;
  const countryFact = countryHolder && drillAt != null && countrySource ? identityCountryAt(countrySource, countryHolder, drillAt) : null;
  const countryTitle = useCountryLabelsAt(runtime.client, track, countryFact ? [{ tag: countryFact.tag, hour: countryFact.hour }] : [], active);
  const drillTitle = !countryHolder ? 'Trade contributors'
    : countryFact ? `${countryTitle.labelAt(countryFact.hour)(countryFact.tag).name}: imports and exports` : 'Imports and exports';

  const description = useMemo(() => {
    if (!data || !diagram.links.length) return message ?? '';
    const sorted = [...diagram.links].sort((a, b) => b.value - a.value);
    const listed = sorted.slice(0, DESCRIBED).map(link => `${nodeName(nodeById.get(link.source)!)} to ${nodeName(nodeById.get(link.target)!)}, ${resourceName(link.resource)} ${amount(link.flows)}`);
    return `${measureNote(measure)}, ${measure === 'current' ? dayLabel(moment) : `${dayLabel(from)} to ${dayLabel(moment)}`}: ${listed.join('; ')}${sorted.length > DESCRIBED ? `; and ${sorted.length - DESCRIBED} more` : ''}.`;
  }, [data, diagram, message, nodeName, nodeById, amount, measure, moment, from]);

  if (!track) return <>{pinnedHeader}<WidgetState kind="empty" icon="flows" message="Load a campaign to see who trades with whom." /></>;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <>{pinnedHeader}<WidgetState kind="unavailable" message="This time is outside the recording." /></>;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Choose another time in widget settings." /></>;
  const groupFailure = (widget.countries.kind === 'group' || marketView && !all) && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return <>{pinnedHeader}{groupFailure}</>;
  if (marketView) {
    if (incoming.marketReason) return <>{groupFailure}<WidgetState kind="unavailable" message={incoming.marketReason} /></>;
    if (incoming.market?.error) return <>{groupFailure}<WidgetState kind="error" failure={incoming.market.error} onRetry={incoming.market.retry} /></>;
    if (!incoming.market?.data || (widget.countries.kind === 'group' && !groups.data)) return <>{groupFailure}<WidgetState kind="loading" label="Loading market and lend-lease…" /></>;
    const pickedMarket = pinnedView ? savedSelection?.kind === 'market' ? savedSelection.key : null : marketSelection;
    if (pinnedView && !pickedMarket) return <>{groupFailure}<WidgetState kind="unavailable" message="This market selection is unavailable. Select it again." /></>;
    const main = <TradeMarketView data={incoming.market.data} onPick={key => {
      setMarketSelection(key); dispatchDrill(openContributions(widget, incoming.market.data!.hour, clock.playing, playhead));
    }} />;
    return <div className="trade-flows" ref={anchor}>{groupFailure}
      <DrillHost mode={!pinnedView && options.drill === 'popout' ? 'popout' : 'inline'} open={effectiveDrill.breakdown != null} anchor={anchor}
        title="Market & lend-lease" day={drillAt == null ? '' : dayLabel(drillAt)} following={effectiveDrill.follow} canFollow
        returnFocus=".market-lendlease summary" onClose={() => { dispatchDrill({ type: 'close' }); if (pinnedView) setOption?.('show', 'main'); }}
        onFollow={() => dispatchDrill(drill.breakdown ? { type: 'follow' } : { type: 'open', breakdown: 'contributions', hour: moment, playing: true, playhead: Math.floor(runtime.clock.getHour()) + track.axisOffset })}
        onPin={drillAt == null || !pickedMarket ? undefined : () => {
          const pinned = pinContributions(widget, track, { kind: 'market', key: pickedMarket }, drillAt, effectiveDrill.follow, extent);
          runtime.addWidgetBelow(widget.id, { ...pinned, title: 'Market & lend-lease', options: { ...pinned.options, view: 'market' } });
        }} main={main}>
        <TradeMarketView data={incoming.market.data} selected={pickedMarket} />
      </DrillHost>
    </div>;
  }
  const selectionReason = selectionMessage(resolved, atMoment, measure, from, moment, groupUnobserved);
  if (selectionReason && !pinnedView && !drill.breakdown) return <>{groupFailure}<WidgetState kind="empty" message={selectionReason} /></>;
  if (error) return <>{groupFailure}{pinnedHeader}<WidgetState kind="error" failure={error} onRetry={retry} /></>;
  // A group with no member found yet asks for nothing, so no reply will come: the moment shown
  // says why. Totals over time can't say so from one moment, and wait for the whole branch.
  const none = measure === 'current' && resolved ? noMembersAt(widget.countries, resolved, groups.data, moment, lastHour) : null;
  if (none && !pinnedView && !drill.breakdown) return <>{groupFailure}{pinnedHeader}<NoMembersState reason={none.reason} icon="flows" /></>;
  if (!data || !trade || (widget.countries.kind === 'group' && !groups.data)) return <>{groupFailure}{pinnedHeader}<WidgetState kind="loading" label="Loading trade…" /></>;
  if ((data as PreparedTradeReply).holdersPending) return <>{groupFailure}{pinnedHeader}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  if (!data.reads.length) return <>{groupFailure}{pinnedHeader}<WidgetState kind="unavailable" message="Trade was not recorded." /></>;
  if (resourceIds && !resourceIds.size) return <>{groupFailure}{pinnedHeader}<WidgetState kind="empty" message="None of the chosen resources is in this campaign. Choose others in the widget settings." /></>;
  if (pinnedView && options.tradeSelection && (!savedSelection || savedSelection.kind === 'market')) return <>{groupFailure}{pinnedHeader}<WidgetState kind="unavailable" message="This trade selection is unavailable. Select trades again." /></>;

  const caption = measure === 'current' ? moment === requestedHour ? 'Per day' : `Per day · As of ${dayLabel(moment)}`
    : `${measure === 'total' ? 'Delivered' : 'Average per trading day'} · ${dayLabel(from)} – ${dayLabel(moment)}`;
  const main = scaleFailure ? <WidgetState kind="error" what="the campaign-wide ribbon scale" failure={scaleFailure} onRetry={incoming.retryScale} />
    : incoming.hasExactHour && !incoming.hasExactHour(moment) ? <WidgetState kind="loading" label="Loading trade amounts…" />
    : scale === undefined ? <WidgetState kind="loading" label="Loading the campaign-wide ribbon scale…" /> : <div className="trade-main">
    <p className="trade-caption">{caption}
      {coverage ? <span> · {coverage}</span> : null}</p>
    <Legend always label="Resources" items={resourceLegend} windowed={shownResources.length > tokens.categories.length} />
    {options.colour === 'seller' ? <p className="chart-line-key">Ribbons in each seller’s map colour</p> : null}
    <div className="trade-inventory-action"><button type="button" className="button small quiet" onClick={() => {
      setSelection(ALL_CONTRIBUTORS);
      dispatchDrill(openContributions(widget, moment, clock.playing, playhead));
    }}>All trades</button></div>
    <FlowDiagram label={`Trade flows: sellers to buyers${all ? ', all countries' : ''}`} nodes={flowNodes} links={flowLinks}
      stretch={measure === 'current' ? spanIndex : 'all'} scale={scale} slots={perSide + 1} hour={moment} onPick={openDrill}
      hoursPerSecond={motionOf(clock)} message={message} tooltip={tooltip} onContextMenu={onContextMenu} description={description} onCapacity={setCapacity}
      onLabelPick={openLabel}
      active={active && !(effectiveDrill.breakdown && (pinnedView || options.drill !== 'popout'))} />
  </div>;
  return <div className="trade-flows" ref={anchor}>{groupFailure}
    {pinnedHeader}
    <DrillHost mode={pinnedView || options.drill !== 'popout' ? 'inline' : 'popout'} open={effectiveDrill.breakdown != null} anchor={anchor}
      title={drillTitle} day={drillAt == null ? '' : dayLabel(drillAt)} following={effectiveDrill.follow} canFollow
      returnFocus=".flow-diagram canvas" onClose={() => {
        dispatchDrill({ type: 'close' });
        if (pinnedView) setOption?.('show', 'main');
      }}
      onFollow={() => dispatchDrill(drill.breakdown ? { type: 'follow' }
        : { type: 'open', breakdown: 'contributions', hour: moment, playing: true, playhead: Math.floor(runtime.clock.getHour()) + track.axisOffset })}
      onPin={chosenSelection && drillAt != null ? () => runtime.addWidgetBelow(widget.id,
        pinContributions(widget, track, chosenSelection, drillAt, effectiveDrill.follow, extent, countryFact?.tag ?? null)) : undefined} main={main}>
      {countryHolder ? countryReply.error && countryKey ? <WidgetState kind="error" failure={countryReply.error} onRetry={countryReply.retry} />
        : !countryData ? <WidgetState kind="loading" label="Loading this country's trade…" />
        // A failed read of one hovered hour says so and can be retried; the country's history stays.
        : <>{countryPoint.error && countryKey && <WidgetState kind="error" what="this hour’s trade" failure={countryPoint.error} onRetry={countryPoint.retry} />}
          <TradeCountryView holder={countryHolder} trade={countryData} prepared={countryPrepared} resources={resourceIds} track={track} runtime={runtime} measure={measure} from={from}
          hour={countryShown} extent={extent} playhead={effectiveDrill.follow ? Math.floor(playbackHour) + track.axisOffset : null}
          colours={resourceColour} tokens={tokens} active={active && effectiveDrill.breakdown != null}
          onPick={hour => dispatchDrill(drill.breakdown ? { type: 'pick', hour } : { type: 'open', breakdown: 'contributions', hour, playing: false, playhead: null })}
          onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} /></>
      : chosenSelection && <TradeDrill flows={contributors} trade={trade} track={track} runtime={runtime} measure={measure} from={from}
        hour={drillShown} extent={extent} playhead={effectiveDrill.follow ? Math.floor(playbackHour) + track.axisOffset : null}
        prepared={incoming} known={known} colours={resourceColour} tokens={tokens} identityNote={scope?.kind === 'countries' ? scope.identityNote : undefined} active={active && effectiveDrill.breakdown != null}
        onPick={hour => dispatchDrill(drill.breakdown ? { type: 'pick', hour } : { type: 'open', breakdown: 'contributions', hour, playing: false, playhead: null })}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />}
    </DrillHost>
    {menu.menu}
  </div>;
}
