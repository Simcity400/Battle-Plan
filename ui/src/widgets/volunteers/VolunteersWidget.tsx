import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime.ts';
import { coverageOf } from '../../dashboard/widgetTime.ts';
import { useDrill } from '../../charts/drill/useDrill';
import FlowDiagram from '../../charts/flow/FlowDiagram';
import type { FlowHit, FlowValues } from '../../charts/flow/FlowDiagram';
import { liveAt } from '../../charts/live';
import { useShownHour } from '../../dashboard/widgetPerf';
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
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { useHolderLooks } from '../../dashboard/holderLooks.ts';
import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import { lookHour } from '../../dashboard/countryLooks';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useHeadHour } from '../army-size/headHour.ts';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS } from './definition';
import { campaignScale, emptyMessage, flowParts, linkFilter, linkHolders, linksAt, partnersOf, requestTags, withoutRequest } from './volunteersData';
import type { LinkAt, VolunteerKind } from './volunteersData';
import './volunteers.css';
import type { VolunteersExpeditionariesReply } from '../../generated/dashboard';
import { useChangedState } from '../../dashboard/changedState.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const NO_TAGS: readonly string[] = [];
const unit = (kind: VolunteerKind, count: number) => kind === 'land' ? `${usNumber.format(count)} ${count === 1 ? 'division' : 'divisions'}`
  : `${usNumber.format(count)} ${count === 1 ? 'division' : 'divisions'} lent`;
type Partner = LinkAt & { partner: string; partnerKey: string; direction: 'sent' | 'received' };

export default function VolunteersWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const linksKey = useRef<(hour: number) => string>(() => '');
  const playbackHour = useHeadHour(runtime.clock, active && widget.time.mode === 'follow', hour => linksKey.current(hour));
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const kind: VolunteerKind = options.kind === 'expeditionary' ? 'expeditionary' : 'land';
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const at = Math.floor(window?.lastHour ?? Number.NaN);
  const gate = useGate(runtime, widget, SPEC, active);
  const all = widget.countries.kind === 'all';
  const groups = useCountryGroups(runtime, track, active && !all);
  const fallbackKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const lastHour = track?.branch.lastHour ?? 0;
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, lastHour), [widget.countries, fallback, groups.data, lastHour]);
  const chosen = useMemo(() => countryMatchFilter(widget.countries, fallback, groups.data, lastHour, undefined, track?.branch.firstHour),
    [widget.countries, fallback, groups.data, lastHour, track?.branch.firstHour]);
  const tags = useMemo(() => requestTags(all, chosen.membership), [all, chosen.membership]);
  const requestable = all || (match.ready && !match.reason && !!tags?.length);
  const drillControl = useDrill(runtime, useRegion(widget));
  const reply = useProduct<VolunteersExpeditionariesReply>(runtime.client, track && (gate.open || drillControl.gate.open) && requestable ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'volunteers-expeditionaries',
    params: { branchId: track.branch.id, kind, countries: tags?.map(countryFromKey) ?? null } }) : null, active, undefined, track?.source);
  const keep = useMemo(() => linkFilter(all ? null : match.matches, options.between === 'between'), [all, match, options.between]);
  const current = useMemo(() => reply.data ? linksAt(reply.data, at, keep) : { links: [], unknown: 0 }, [reply.data, at, keep]);
  linksKey.current = hour => {
    if (!reply.data || !track) return '';
    const shown = Math.floor(resolveWindow(widget, track, runtime.tracks, hour)?.lastHour ?? track.branch.lastHour);
    const held = linksAt(reply.data, shown, keep);
    return JSON.stringify([held.unknown, held.links.map(link => link.key)]);
  };
  const [capacity, setCapacity] = useState(12);
  const perSide = Math.max(1, Math.min(Number(options.perSide) || 12, capacity - 1));
  const summed = useMemo(() => sumFlowParts(flowParts(current.links), perSide), [current.links, perSide]);
  const boundaries = useMemo(() => (chosen.membership ?? []).map(run => run.firstHour), [chosen.membership]);
  const scale = useMemo(() => options.scale === 'moment' || !reply.data ? null : campaignScale(reply.data, keep, boundaries) || null, [options.scale, reply.data, keep, boundaries]);
  const holders = useMemo(() => linkHolders(current.links), [current.links]);
  const tagOf = useMemo(() => new Map(holders.map(holder => [holder.key, holder.tag])), [holders]);
  const looks = useHolderLooks(runtime.client, track, holders, active, tokens);
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const picked = drill.breakdown;
  const pickedTag = useMemo(() => { if (!picked) return null; const parsed: unknown = JSON.parse(picked); return Array.isArray(parsed) && typeof parsed[0] === 'string' ? parsed[0] : null; }, [picked]);
  const [drillRows, setDrillRows] = useChangedState<readonly string[]>([]);
  const drillListed = picked !== null && current.links.some(item => item.senderKey === picked || item.hostKey === picked);
  const drillAt = drillControl.hour;
  const nodeTags = useMemo(() => [...new Set(summed.nodes.flatMap(node => node.tag ? [tagOf.get(node.tag)!] : []))], [summed.nodes, tagOf]);
  const partnerTags = drillListed || picked !== null ? drillRows : NO_TAGS;
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => [...holders.map(holder => ({ hour: at, tag: holder.tag })),
    ...(pickedTag && drillAt !== null ? [{ hour: drillAt, tag: pickedTag }, ...partnerTags.map(tag => ({ hour: drillAt, tag }))] : [])],
  [holders, at, pickedTag, drillAt, partnerTags]), active);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, useMemo(() => [...nodeTags.map(tag => ({ hour: at, tag })),
    ...(drillAt === null ? [] : partnerTags.map(tag => ({ hour: drillAt, tag })))], [nodeTags, at, partnerTags, drillAt]), active);
  const label = useCallback((tag: string, hour = at) => ({ ...labelAt(hour)(tag), ...(names.has(nameKey(hour, tag)) ? nameReadMetadata(names.get(nameKey(hour, tag))) : {}) }), [labelAt, at, names]);
  const colours = useMemo(() => countryFlowColours(holders.map(holder => holder.key), key => looks.color(key), tokens.surface, tokens.countryColours === 'fixed'),
    [holders, looks, tokens.surface, tokens.countryColours]);
  const nodeName = useCallback((node: SummedNode) => node.tag ? label(tagOf.get(node.tag)!).name : `Other (${node.members.length})`, [label, tagOf]);
  const { nodes, colours: nodeColours } = useMemo(() => countryFlowNodes(summed.nodes, key => label(tagOf.get(key)!), tokens, nodeName, node => node.tag ?? '', colours),
    [summed.nodes, label, tagOf, tokens, nodeName, colours]);
  const links = useMemo(() => summed.links.map(link => ({ id: link.id, source: link.source, target: link.target, value: link.value,
    color: nodeColours.get(link.source) ?? tokens.muted, order: link.id })), [summed.links, nodeColours, tokens.muted]);
  const valuesAt = useCallback((hour: number): FlowValues => {
    const held = reply.data ? linksAt(reply.data, hour, keep) : { links: [], unknown: 0 };
    const sum = sumFlowParts(flowParts(held.links), perSide);
    const made = countryFlowNodes(sum.nodes, key => label(tagOf.get(key) ?? key), tokens, nodeName, node => node.tag ?? '', colours);
    return { nodes: made.nodes, hour, links: sum.links.map(link => ({ id: link.id, source: link.source, target: link.target, value: link.value,
      color: made.colours.get(link.source) ?? tokens.muted, order: link.id })) };
  }, [reply.data, keep, perSide, label, tagOf, tokens, nodeName, colours]);
  const following = active && widget.time.mode === 'follow' && !!track;
  const live = useMemo(() => following && track ? liveAt(runtime.clock,
    position => Math.floor(resolveWindow(widget, track, runtime.tracks, position)?.lastHour ?? track.branch.lastHour), valuesAt) : null,
  [following, track, runtime.clock, runtime.tracks, widget, valuesAt]);
  const shown = useShownHour();
  const reported = useMemo(() => live ? { ...live, read: () => { const value = live.read(); shown(value.hour ?? null); return value; } } : undefined, [live, shown]);
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
  const drillLinks = useMemo(() => reply.data && drillAt !== null ? linksAt(reply.data, drillAt, keep).links : [], [reply.data, drillAt, keep]);
  const partners = useMemo<Partner[]>(() => { if (!picked) return []; const { sent, received } = partnersOf(drillLinks, picked);
    return [...sent.map(link => ({ ...link, partner: link.host, partnerKey: link.hostKey, direction: 'sent' as const })),
      ...received.map(link => ({ ...link, partner: link.sender, partnerKey: link.senderKey, direction: 'received' as const }))]; }, [picked, drillLinks]);
  const pickedName = pickedTag && drillAt !== null ? label(pickedTag, drillAt).name : '';
  const partnerLabel = useCallback((tag: string) => drillAt === null ? { name: tag } : label(tag, drillAt), [label, drillAt]);
  const columns = useMemo<readonly TableColumn<Partner>[]>(() => [
    { key: 'partner', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 2fr)', value: row => partnerLabel(row.partner).name,
      render: row => <CountryCell {...partnerLabel(row.partner)} /> },
    { key: 'direction', header: 'Direction', label: 'Direction', width: 'max-content', priority: 1, value: row => row.direction, render: row => row.direction === 'sent' ? 'Sent to' : 'Received from' },
    { key: 'divisions', header: 'Divisions', label: 'Divisions', numeric: true, width: 'max-content', value: row => row.divisions, render: row => usNumber.format(row.divisions) },
    { key: 'since', header: 'Since', label: 'Since', width: 'max-content', priority: 2, value: row => row.since, render: row => dayLabel(row.since) },
  ], [partnerLabel]);
  if (!track) return <WidgetState kind="empty" icon="flows" message="Load a campaign to see volunteers." />;
  const groupFailure = !all && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  const unrequested = !all && groups.data && !tags?.length ? withoutRequest({ hourly: widget.countries.kind === 'group' && widget.countries.membership === 'hour',
    membershipUnobserved: chosen.unobserved.some(span => span.firstHour <= at && at <= span.lastHour) }) : null;
  const parentState = gate.outside ? <WidgetState kind="outside" read={coverageOf(gate.outside)} />
    : groupFailure && !groups.data ? groupFailure
    : match.reason ? <WidgetState kind="empty" message={match.reason} />
    : unrequested ? <WidgetState kind={unrequested.kind} message={unrequested.message} />
    : !all && !match.ready ? <WidgetState kind="loading" label="Loading volunteers…" />
    : reply.error ? <WidgetState kind="error" failure={reply.error} onRetry={reply.retry} />
    : !reply.data ? <WidgetState kind="loading" label="Loading volunteers…" /> : null;
  if (parentState && picked === null) return <>{groupFailure && parentState !== groupFailure ? groupFailure : null}{parentState}</>;
  const covers = (span: { firstHour: number; lastHour: number }) => span.firstHour <= at && at <= span.lastHour;
  const message = current.links.length ? null : emptyMessage(kind, { aidUnobserved: reply.data?.unobserved.some(covers) ?? false,
    membershipUnobserved: chosen.unobserved.some(covers), unknown: current.unknown });
  const main = parentState ?? <FlowDiagram live={reported} label={`${kind === 'land' ? 'Volunteers' : 'Expeditionary forces'}: senders to hosts`} nodes={nodes} links={links}
    stretch={`${kind}:${lookHour(track.branch, at)?.stretch ?? 'unrecorded'}`} scale={scale} slots={perSide + 1} hour={at} hoursPerSecond={hoursPerSecond} message={message} tooltip={tooltip}
    onCapacity={setCapacity} onLabelPick={node => { const found = summed.nodes.find(item => item.id === node.id); if (found?.tag) dispatch({ type: 'open', breakdown: found.tag, hour: at, follow: false }); }}
    onContextMenu={(hit, x, y) => { const link = hit?.kind === 'link' ? summed.links.find(item => item.id === hit.link.id) : null;
      const since = link ? Math.min(...link.parts.map(part => part.since)) : null;
      if (since != null) menu.open(x, y, [{ ...jumpItem(runtime, track.id, since), label: `Jump to the start ${dayLabel(since)}` }]); }}
    description={`${current.links.length} links at ${dayLabel(at)}.`} active={active && (!picked || options.drill === 'popout')} />;
  return <div ref={anchor} className="volunteers">{groupFailure}
    <DrillHost drill={drillControl} runtime={runtime} subject={picked != null} mode={options.drill === 'popout' ? 'popout' : 'inline'} title={pickedName} day={drillAt === null ? '' : dayLabel(drillAt)} canFollow={false} anchor={anchor} onClose={() => { dispatch({ type: 'close' }); setDrillRows([]); }} main={main}>
      {picked && (partners.length ? <Table rowRevision={row => [row, columns]} label={`${pickedName} ${kind === 'land' ? 'volunteers' : 'expeditionary forces'}`} rows={partners} columns={columns}
        rowKey={row => `${row.direction}:${row.key}`} sort={{ column: 'divisions', direction: 'descending' }} cellBars={false} rankMarkers={false}
        onVisibleRows={rows => setDrillRows(previous => { const next = [...new Set(rows.map(row => row.partner))];
          return previous.length === next.length && previous.every((tag, index) => tag === next[index]) ? previous : next; })} />
        : reply.data ? <WidgetState kind="empty" message="Nothing sent or received at this date." />
          : reply.error ? <WidgetState kind="error" failure={reply.error} onRetry={reply.retry} /> : <WidgetState kind="loading" label="Loading volunteers…" />)}
    </DrillHost>
    {menu.menu}
  </div>;
}
