import { useCallback, useEffect, useMemo, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import FlowDiagram from '../../charts/flow/FlowDiagram';
import type { FlowHit, FlowLink, FlowNode } from '../../charts/flow/FlowDiagram';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { nudgeApart } from '../../charts/palette';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount } from '../../charts/valueAxis';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { readAt } from '../../dashboard/reads';
import { mix, parseColor, toHex } from '../../shell/appearance/color';
import { buildDiagram, campaignScale, coverageNote, sideRanks, decodeTrade, exactSums, formatDays, knownHours, emptyMessage, knownSpanAt, unrecordedMessage, measureAmount, measureNote, OTHER, resourceColours, selectionMessage,
  resourceName, selectFlows } from './tradeData';
import type { Flow, Measure, SideNode, TradeScope } from './tradeData';
import { OPTIONS } from './definition';
import { useTradeFlows } from './useTradeFlows';
import { nodeTooltip, ribbonTooltip, TOOLTIP_LIMIT } from './tradeTooltip';
import './tradeFlows.css';

const NO_WARNINGS: string[] = [];
/** Ribbons the screen-reader description lists, largest first. */
const DESCRIBED = 24;

type Label = { name: string; flag?: string; color: readonly [number, number, number] | null };

/** Who sells what to whom (mock-up style-6): a Sankey of sellers to buyers, one
 * ribbon per seller, buyer and resource, sized by the chosen measure. */
export default function TradeFlowsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = (options.measure as Measure) ?? 'current';
  const asked = Number(options.perSide ?? 12);
  // As many countries a side as the diagram fits at this size (the flow kit says),
  // one slot kept for the rest summed.
  const [capacity, setCapacity] = useState<number | null>(null);
  const perSide = capacity == null ? asked : Math.max(1, Math.min(asked, capacity - 1));
  const { track, all, resolved, groups, data, error, retry } = useTradeFlows(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const from = extent[0];
  const menu = useContextMenu('Trade actions');

  const trade = useMemo(() => data && decodeTrade(data), [data]);
  const chosen = contentSet(options.resources);
  const chosenKey = JSON.stringify(chosen);
  const resourceIds = useMemo(() => {
    if (!trade || !chosen.length) return null;
    return new Set(trade.reply.resources.filter(item => chosen.includes(item.token)).map(item => item.id));
  }, [trade, chosenKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const between = options.between === 'between';
  const scope = useMemo<TradeScope>(() => all || !resolved ? { kind: 'all' }
    : { kind: 'countries', tags: resolved.tags, membership: resolved.membership, unobserved: resolved.unobserved, between }, [all, resolved, between]);
  const flows = useMemo(() => trade ? selectFlows(trade, scope, resourceIds) : [], [trade, scope, resourceIds]);
  const diagram = useMemo(() => buildDiagram(flows, measure, from, moment, perSide), [flows, measure, from, moment, perSide]);
  const scale = useMemo(() => options.scale === 'moment' || !data ? null : campaignScale(flows, measure, from, extent[1]),
    [flows, measure, from, extent[1], options.scale, data]); // eslint-disable-line react-hooks/exhaustive-deps
  // Each country's fixed place for ribbons inside the other side's nodes.
  const ranks = useMemo(() => sideRanks(flows, extent[0], extent[1]), [flows, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps

  // Countries named, flagged and coloured as recorded near the moment shown, under the
  // kit's looks rule (`useCountryLooks`): read about once a month of playback; a look
  // is held only while the next read loads, and a part a finished read did not give
  // is not known, never an older one.
  const read = data ? readAt(data.reads, moment) : null;
  const clock = runtime.clock.getSnapshot();
  const lookTags = useMemo(() => diagram.nodes.flatMap(node => node.tag ? [node.tag] : node.members.slice(0, TOOLTIP_LIMIT)), [diagram]);
  const looks = useCountryLooks(runtime.client, track, lookTags, moment, active);
  const labelOf = useCallback((tag: string): Label => ({ name: looks.name(tag), flag: looks.flag(tag), color: looks.color(tag) }), [looks]);
  // A tooltip names its countries as recorded at the hour it is about (`useHoverLabels`,
  // only the tooltip's countries asked for); the diagram's looks show while they load.
  const pending = useCallback((tag: string) => ({ name: looks.name(tag), flag: looks.flag(tag) }), [looks]);
  const hover = useHoverLabels(runtime.client, track, active, pending);

  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, onDiagnostics]);

  // What the diagram draws: nodes named and coloured, ribbons by resource or seller.
  const shownResources = useMemo(() => {
    const ids = new Set(flows.map(flow => flow.resource.id));
    return trade ? trade.reply.resources.filter(item => ids.has(item.id)) : [];
  }, [flows, trade]);
  const otherColour = mix(tokens.surface, tokens.text, 0.4);
  const resourceColour = useMemo(() => trade ? resourceColours(trade.reply.resources, trade.resourceOrder, tokens.categories, otherColour) : new Map<number, string>(),
    [trade, tokens.categories, otherColour]);
  const nodeName = useCallback((node: SideNode) => node.tag ? labelOf(node.tag).name
    : `${node.members.length.toLocaleString('en-US')} other ${node.side === 'source' ? (node.members.length === 1 ? 'seller' : 'sellers') : (node.members.length === 1 ? 'buyer' : 'buyers')}`, [labelOf]);
  const { flowNodes, flowLinks } = useMemo(() => {
    const countries = diagram.nodes.filter(node => node.tag);
    const recorded = countries.map(node => { const color = labelOf(node.tag!).color; return color ? { r: color[0], g: color[1], b: color[2] } : null; });
    const surface = parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 };
    const fixed = tokens.countryColours === 'fixed' ? nudgeApart(recorded.map(color => color ?? surface), surface) : null;
    const colour = new Map(countries.map((node, index) => [node.id, recorded[index] ? toHex(fixed ? fixed[index] : recorded[index]!) : otherColour]));
    const flowNodes: FlowNode[] = diagram.nodes.map(node => ({ id: node.id, side: node.side, label: nodeName(node), color: colour.get(node.id) ?? otherColour,
      flag: node.tag ? labelOf(node.tag).flag ?? null : undefined,
      order: node.tag ? String(ranks[node.side].get(node.tag) ?? 0).padStart(7, '0') : OTHER, last: !node.tag }));
    const order = trade?.resourceOrder;
    const flowLinks: FlowLink[] = diagram.links.map(link => ({ id: link.id, source: link.source, target: link.target, value: link.value,
      color: options.colour === 'seller' ? colour.get(link.source) ?? otherColour : resourceColour.get(link.resource.id) ?? otherColour,
      order: String(order?.get(link.resource.id) ?? 0).padStart(6, '0') }));
    return { flowNodes, flowLinks };
  }, [diagram, labelOf, nodeName, tokens, otherColour, resourceColour, options.colour, trade, ranks]);

  const spanIndex = data ? knownSpanAt(data.known, moment) : -1;
  // The countries at the moment shown, by the kit's whole-world rule (`worldFilter`):
  // its reason says when a group's members there are unknown or none. Totals over
  // time of a group counted at each hour count every member while it was one.
  const lastHour = track?.branch.lastHour ?? 0;
  const fallback = defaultCountryTag(track);
  const atMoment = worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, moment, lastHour);
  const momentReason = atMoment.reason && (measure === 'current' || !resolved?.membership) ? atMoment.reason : null;
  const groupUnobserved = widget.countries.kind === 'group' && widget.countries.membership === 'hour' ? resolved?.unobserved ?? null : null;
  // Nothing recorded is said as such for every measure, never as no trade; deals a read
  // could not attribute leave the hour known only in part, which is said too.
  const unrecorded = data ? unrecordedMessage(data.known, measure, from, moment) : null;
  const message = !data ? null : unrecorded
    ?? momentReason ?? (diagram.links.length ? null : emptyMessage(data, measure, from, moment, groupUnobserved));
  // Totals and averages cover recorded hours only: unrecorded deals, and hours a group
  // counted at each hour has no recorded membership, are said, never counted as none.
  const groupHours = groupUnobserved ? knownHours(groupUnobserved, from, moment) : 0;
  const coverage = data ? [coverageNote(data, measure, from, moment),
    measure !== 'current' && groupHours >= 1 ? `${formatDays(groupHours)} of the group’s membership in this time were not recorded; they are not counted.` : undefined]
    .filter(Boolean).join(' ') || undefined : undefined;
  const note = coverage;
  const nodeById = useMemo(() => new Map(diagram.nodes.map(node => [node.id, node])), [diagram]);
  const linkById = useMemo(() => new Map(diagram.links.map(link => [link.id, link])), [diagram]);

  const amount = useCallback((list: readonly Flow[]) => data ? formatAmount(measureAmount(list, measure, from, moment), data.decimals) : '', [data, measure, from, moment]);
  const tooltip = useCallback((hit: FlowHit): TooltipContent | null => {
    if (!data) return null;
    const tooltipOptions = { measure, from, hour: moment, read, decimals: data.decimals, note, viewport,
      nodeName, labelAt: hover.labelAt, ask: hover.ask };
    if (hit.kind === 'link') {
      const link = linkById.get(hit.link.id);
      if (!link) return null;
      const [source, target] = [nodeById.get(link.source)!, nodeById.get(link.target)!];
      return ribbonTooltip(link, source, target, tooltipOptions);
    }
    const node = nodeById.get(hit.node.id);
    if (!node) return null;
    return nodeTooltip(node, diagram.links, { ...tooltipOptions, resourceOrder: trade?.resourceOrder,
      resourceColour: id => options.colour === 'resource' ? resourceColour.get(id) : undefined });
  }, [data, trade, measure, moment, from, read, hover.labelAt, hover.ask, linkById, nodeById, nodeName, note, viewport, diagram, options.colour, resourceColour]);

  const onContextMenu = useCallback((hit: FlowHit | null, x: number, y: number) => {
    if (!track) return;
    // The moment shown, as read for the current amount; a ribbon adds when it was
    // first and last recorded in the time shown.
    const items: MenuItem[] = [jumpItem(runtime, track.id, measure === 'current' ? read ?? moment : moment)];
    const link = hit?.kind === 'link' ? linkById.get(hit.link.id) : null;
    if (link) {
      const sums = exactSums(link.flows, extent[0], extent[1]);
      if (sums.first != null) items.push({ ...jumpItem(runtime, track.id, sums.first), hint: 'First recorded in the time shown' });
      if (sums.last != null && sums.last !== sums.first) items.push({ ...jumpItem(runtime, track.id, sums.last), hint: 'Last recorded in the time shown' });
    }
    menu.open(x, y, items);
  }, [track, runtime, measure, read, moment, linkById, extent[0], extent[1], menu]); // eslint-disable-line react-hooks/exhaustive-deps

  const description = useMemo(() => {
    if (!data || !diagram.links.length) return message ?? '';
    const sorted = [...diagram.links].sort((a, b) => b.value - a.value);
    const listed = sorted.slice(0, DESCRIBED).map(link => `${nodeName(nodeById.get(link.source)!)} to ${nodeName(nodeById.get(link.target)!)}, ${resourceName(link.resource)} ${amount(link.flows)}`);
    return `${measureNote(measure)}, ${measure === 'current' ? dayLabel(moment) : `${dayLabel(from)} to ${dayLabel(moment)}`}: ${listed.join('; ')}${sorted.length > DESCRIBED ? `; and ${sorted.length - DESCRIBED} more` : ''}.`;
  }, [data, diagram, message, nodeName, nodeById, amount, measure, moment, from]);

  if (!track) return <WidgetState kind="empty" icon="flows" message="Load a campaign to see who trades with whom." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (widget.countries.kind === 'group' && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  const selectionReason = selectionMessage(resolved, atMoment, measure, from, moment, groupUnobserved);
  if (selectionReason) return <WidgetState kind="empty" message={selectionReason} />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || !trade || (widget.countries.kind === 'group' && !groups.data)) return <WidgetState kind="loading" label="Loading trade…" />;
  if (!data.reads.length) return <WidgetState kind="unavailable" message="No trade deals were recorded in this branch." />;
  if (resourceIds && !resourceIds.size) return <WidgetState kind="empty" message="None of the chosen resources is in this campaign. Choose others in the widget settings." />;

  const caption = measure === 'current' ? `Per day${read != null && spanIndex >= 0 ? ` · read ${formatDateTime(read)}` : ''}`
    : `${measure === 'total' ? 'Delivered' : 'Average per day while trading'} · ${dayLabel(from)} – ${dayLabel(moment)}`;
  return <div className="trade-flows">
    <p className="trade-caption">{caption}{perSide < asked ? ` · ${perSide} countries a side fit this size; the rest are summed` : ''}
      {coverage ? <span> · {coverage}</span> : null}</p>
    {options.colour === 'resource'
      ? <Legend always label="Resources" items={shownResources.map(item => ({ id: String(item.id), label: resourceName(item), color: resourceColour.get(item.id) ?? otherColour,
        flag: item.icon ? <img className="trade-resource-icon" src={item.icon} alt="" /> : <span className="trade-resource-icon is-missing" aria-hidden="true" /> }))} />
      : <p className="chart-line-key">Ribbons in each seller’s map colour</p>}
    <FlowDiagram label={`Trade flows: sellers to buyers${all ? ', all countries' : ''}`} nodes={flowNodes} links={flowLinks}
      stretch={measure === 'current' ? spanIndex : 'all'} scale={scale} slots={perSide + 1}
      hoursPerSecond={clock.hoursPerSecond} message={message} tooltip={tooltip} onContextMenu={onContextMenu} description={description} onCapacity={setCapacity}
      active={active} />
    {menu.menu}
  </div>;
}
