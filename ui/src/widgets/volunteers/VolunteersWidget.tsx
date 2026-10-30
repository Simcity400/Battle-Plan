import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import FlowDiagram from '../../charts/flow/FlowDiagram';
import type { FlowHit } from '../../charts/flow/FlowDiagram';
import { countryFlowColours, countryFlowNodes } from '../../charts/flow/countries';
import { sumFlowParts } from '../../charts/flow/sum';
import type { SummedNode } from '../../charts/flow/sum';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/columns';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useContextMenu } from '../../components/ContextMenu';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesRead } from '../../dashboard/countryNamesAt';
import { nameReadText, useHolderLooks } from '../../dashboard/holderLooks';
import { pendingAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { lookHour } from '../../dashboard/countryLooks';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { KINDS, OPTIONS } from './definition';
import { campaignScale, emptyMessage, flowParts, linkFilter, linkHolders, linksAt, partnersOf, requestTags, withoutRequest } from './volunteersData';
import type { LinkAt, VolunteerKind } from './volunteersData';
import type { VolunteersExpeditionariesReply } from '../../generated/dashboard';
import './volunteers.css';

const NO_WARNINGS: string[] = [];
const unit = (kind: VolunteerKind, count: number) => kind === 'land' ? `${usNumber.format(count)} ${count === 1 ? 'division' : 'divisions'}`
  : `${usNumber.format(count)} ${count === 1 ? 'division' : 'divisions'} lent`;
type Partner = LinkAt & { partner: string; partnerKey: string; direction: 'sent' | 'received' };

export default function VolunteersWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const kind: VolunteerKind = options.kind === 'expeditionary' ? 'expeditionary' : 'land';
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const at = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  // The country choice is the shared one (`countryMatch`): countries by their native holders
  // and recorded identity history, so a tag a later country takes, or a rename, never changes
  // whose forces count; a group by hour counts a member only while it belongs.
  const all = widget.countries.kind === 'all';
  const groups = useCountryGroups(runtime, track, active && !all);
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const lastHour = track?.branch.lastHour ?? 0;
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, lastHour), [widget.countries, fallback, groups.data, lastHour]);
  const chosen = useMemo(() => countryMatchFilter(widget.countries, fallback, groups.data, lastHour, undefined, track?.branch.firstHour),
    [widget.countries, fallback, groups.data, lastHour, track?.branch.firstHour]);
  // The request names every tag the chosen holders carry in the branch; only All asks for every link.
  const tags = useMemo(() => requestTags(all, chosen.membership), [all, chosen.membership]);
  const requestable = all || (match.ready && !match.reason && !!tags?.length);
  const reply = useProduct<VolunteersExpeditionariesReply>(runtime.client, track && requestable ? { sourceId: track.source.id, revision: track.source.revision, product: 'volunteers-expeditionaries',
    params: { branchId: track.branch.id, kind, tags } } : null, active, undefined, track?.source);
  const keep = useMemo(() => linkFilter(all ? null : match.matches, options.between === 'between'), [all, match, options.between]);
  const current = useMemo(() => reply.data ? linksAt(reply.data, at, keep) : { links: [], unknown: 0 }, [reply.data, at, keep]);
  const [capacity, setCapacity] = useState(12);
  // The combined Other node takes one of the rows the diagram has room for.
  const perSide = Math.max(1, Math.min(Number(options.perSide) || 12, capacity - 1));
  const summed = useMemo(() => sumFlowParts(flowParts(current.links), perSide), [current.links, perSide]);
  // Totals also change where a chosen holder starts carrying a tag or joins the group.
  const boundaries = useMemo(() => (chosen.membership ?? []).map(run => run.firstHour), [chosen.membership]);
  const scale = useMemo(() => options.scale === 'moment' || !reply.data ? null : campaignScale(reply.data, keep, boundaries) || null, [options.scale, reply.data, keep, boundaries]);
  // Nodes are native holders (`holderKey`); each holder is named by its tag at the hour shown,
  // when its link is in place, and coloured as recorded at its links' first hour, a stable hour
  // it held the tag, so a later holder of the tag never lends its look.
  const holders = useMemo(() => linkHolders(current.links), [current.links]);
  const tagOf = useMemo(() => new Map(holders.map(holder => [holder.key, holder.tag])), [holders]);
  const looks = useHolderLooks(runtime.client, track, holders, active, tokens);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  // The drill holds the holder clicked, named as it was at the hour clicked.
  const picked = drill.breakdown;
  const pickedTag = useMemo(() => { if (!picked) return null; const parsed: unknown = JSON.parse(picked); return Array.isArray(parsed) && typeof parsed[0] === 'string' ? parsed[0] : null; }, [picked]);
  const [drillRows, setDrillRows] = useState<readonly string[]>([]);
  // Flags for drawn nodes, and for the drill's rows while its list is mounted.
  const drillListed = picked !== null && current.links.some(item => item.senderKey === picked || item.hostKey === picked);
  const nodeTags = useMemo(() => [...new Set([...summed.nodes.flatMap(node => node.tag ? [tagOf.get(node.tag)!] : []), ...(drillListed ? drillRows : [])])],
    [summed.nodes, tagOf, drillListed, drillRows]);
  // Names for every partner (tooltips list Other's members); flags only for drawn nodes.
  const names = useCountryNamesRead(runtime.client, track, useMemo(() => [...holders.map(holder => ({ hour: at, tag: holder.tag })),
    ...(pickedTag && drill.hour !== null ? [{ hour: drill.hour, tag: pickedTag }] : [])], [holders, at, pickedTag, drill.hour]), active);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, useMemo(() => nodeTags.map(tag => ({ hour: at, tag })), [nodeTags, at]), active);
  const label = useCallback((tag: string, hour = at) => ({ ...labelAt(hour)(tag), name: nameReadText(names.get(nameKey(hour, tag))) }), [labelAt, at, names]);
  const colours = useMemo(() => countryFlowColours(holders.map(holder => holder.key), key => looks.color(key), tokens.surface, tokens.countryColours === 'fixed'),
    [holders, looks, tokens.surface, tokens.countryColours]);
  const nodeName = useCallback((node: SummedNode) => node.tag ? label(tagOf.get(node.tag)!).name : `Other (${node.members.length})`, [label, tagOf]);
  const { nodes, colours: nodeColours } = useMemo(() => countryFlowNodes(summed.nodes, key => label(tagOf.get(key)!), tokens, nodeName, node => node.tag ?? '', colours),
    [summed.nodes, label, tagOf, tokens, nodeName, colours]);
  const links = useMemo(() => summed.links.map(link => ({ id: link.id, source: link.source, target: link.target, value: link.value,
    color: nodeColours.get(link.source) ?? tokens.muted, order: link.id })), [summed.links, nodeColours, tokens.muted]);
  const diagnostics = useMemo(() => [...(reply.data?.warnings ?? []), ...(current.unknown ? [`${current.unknown} links have an unknown count at this time.`] : []),
    ...(reply.error ? [reply.error] : [])], [reply.data, current.unknown, reply.error]);
  useDiagnostics(onDiagnostics, diagnostics.length ? diagnostics : NO_WARNINGS);
  const tooltip = useCallback((hit: FlowHit): TooltipContent | null => {
    if (hit.kind === 'link') {
      const link = summed.links.find(item => item.id === hit.link.id);
      if (!link) return null;
      return { title: `${nodeName(summed.nodes.find(node => node.id === link.source)!)} → ${nodeName(summed.nodes.find(node => node.id === link.target)!)}`,
        subtitle: dayLabel(at), scrollable: link.parts.length > 12, scrollId: link.id, rows: link.parts.map(part => ({
          label: `${label(part.sender).name} → ${label(part.host).name}`, value: `${unit(kind, part.divisions)} · since ${dayLabel(part.since)}` })) };
    }
    const node = summed.nodes.find(item => item.id === hit.node.id);
    if (!node) return null;
    const own = current.links.filter(link => node.members.includes(node.side === 'source' ? link.senderKey : link.hostKey));
    return { title: nodeName(node), countryFlag: node.tag ? label(tagOf.get(node.tag)!) : undefined, subtitle: `${node.side === 'source' ? 'Sends' : 'Hosts'} · ${dayLabel(at)}`,
      scrollable: own.length > 12, scrollId: node.id, rows: [{ label: 'Total', value: unit(kind, node.value), strong: true },
        ...own.map(link => ({ label: node.side === 'source' ? `To ${label(link.host).name}` : `From ${label(link.sender).name}`, value: unit(kind, link.divisions), indent: true }))] };
  }, [summed, current.links, nodeName, label, tagOf, kind, at]);
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Volunteer actions');
  const partners = useMemo<Partner[]>(() => { if (!picked) return []; const { sent, received } = partnersOf(current.links, picked);
    return [...sent.map(link => ({ ...link, partner: link.host, partnerKey: link.hostKey, direction: 'sent' as const })),
      ...received.map(link => ({ ...link, partner: link.sender, partnerKey: link.senderKey, direction: 'received' as const }))]; }, [picked, current.links]);
  const pickedName = pickedTag && drill.hour !== null ? label(pickedTag, drill.hour).name : '';
  const columns = useMemo<readonly TableColumn<Partner>[]>(() => [
    { key: 'partner', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 2fr)', value: row => label(row.partner).name, render: row => <CountryCell {...label(row.partner)} /> },
    { key: 'direction', header: 'Direction', label: 'Direction', width: 'max-content', priority: 1, value: row => row.direction, render: row => row.direction === 'sent' ? 'Sent to' : 'Received from' },
    { key: 'divisions', header: 'Divisions', label: 'Divisions', numeric: true, width: 'max-content', value: row => row.divisions, render: row => usNumber.format(row.divisions) },
    { key: 'since', header: 'Since', label: 'Since', width: 'max-content', priority: 2, value: row => row.since, render: row => dayLabel(row.since) },
  ], [label]);
  if (!track) return <WidgetState kind="empty" icon="flows" message="Load a campaign to see volunteers." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  const groupFailure = !all && groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (match.reason) return <>{groupFailure}<WidgetState kind="empty" message={match.reason} /></>;
  if (!all && groups.data && !tags?.length) {
    const state = withoutRequest({ known: match.known(at), hourly: widget.countries.kind === 'group' && widget.countries.membership === 'hour',
      membershipUnobserved: chosen.unobserved.some(span => span.firstHour <= at && at <= span.lastHour) });
    // Failed preparation cannot finish pending membership, so show the failure instead of loading.
    return state.kind === 'loading' ? groupFailure ?? <WidgetState kind="loading" label={state.message} />
      : <>{groupFailure}<WidgetState kind={state.kind} message={state.message} /></>;
  }
  if (!all && !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading volunteers…" /></>;
  if (reply.error) return <>{groupFailure}<WidgetState kind="error" detail={reply.error} onRetry={reply.retry} /></>;
  if (!reply.data) return <>{groupFailure}<WidgetState kind="loading" label="Loading volunteers…" /></>;
  if (!current.links.length && pendingAt(preparedPrefix(reply.data), at)) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  const covers = (span: { firstHour: number; lastHour: number }) => span.firstHour <= at && at <= span.lastHour;
  const message = current.links.length ? null : emptyMessage(kind, { aidUnobserved: reply.data.unobserved.some(covers),
    membershipUnobserved: chosen.unobserved.some(covers), membershipPending: !match.known(at), unknown: current.unknown });
  // A new kind, or a stretch of the recording after a gap, jumps rather than glides; a tag
  // passing to another country is another node, so it never glides either.
  const main = <FlowDiagram label={`${kind === 'land' ? 'Volunteers' : 'Expeditionary forces'}: senders to hosts`} nodes={nodes} links={links}
    stretch={`${kind}:${lookHour(track.branch, at)?.stretch ?? 'unrecorded'}`} scale={scale} slots={perSide + 1} hour={at} hoursPerSecond={hoursPerSecond} message={message} tooltip={tooltip}
    onCapacity={setCapacity} onLabelPick={node => { const found = summed.nodes.find(item => item.id === node.id); if (found?.tag) dispatch({ type: 'open', breakdown: found.tag, hour: at, playing: false, playhead: null }); }}
    onContextMenu={(hit, x, y) => { const link = hit?.kind === 'link' ? summed.links.find(item => item.id === hit.link.id) : null;
      const since = link ? Math.min(...link.parts.map(part => part.since)) : null;
      if (since != null) menu.open(x, y, [{ ...jumpItem(runtime, track.id, since), label: `Jump to the start ${dayLabel(since)}` }]); }}
    description={`${current.links.length} links at ${dayLabel(at)}.`} active={active && (!picked || options.drill === 'popout')} />;
  return <div ref={anchor} className="volunteers">{groupFailure}
    {setOption && <div className="volunteers-caption"><Segmented value={kind} options={KINDS} label="Kind" onChange={value => setOption('kind', value)} /></div>}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={picked != null} title={pickedName} day={dayLabel(at)}
      following={false} canFollow={false} anchor={anchor} onClose={() => { dispatch({ type: 'close' }); setDrillRows([]); }} onFollow={() => {}} main={main}>
      {picked && (partners.length ? <Table label={`${pickedName} ${kind === 'land' ? 'volunteers' : 'expeditionary forces'}`} rows={partners} columns={columns}
        rowKey={row => `${row.direction}:${row.key}`} sort={{ column: 'divisions', direction: 'descending' }} cellBars={false} rankMarkers={false}
        onVisibleRows={rows => setDrillRows(previous => { const next = [...new Set(rows.map(row => row.partner))];
          return previous.length === next.length && previous.every((tag, index) => tag === next[index]) ? previous : next; })} />
        : <WidgetState kind="empty" message="Nothing sent or received at this date." />)}
    </DrillHost>
    {menu.menu}
  </div>;
}
