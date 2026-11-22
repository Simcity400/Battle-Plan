import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { failureDetails } from '../../dashboard/queryError.ts';
import { useCallback, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import Timeline from '../../charts/timeline/Timeline';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import type { TooltipContent } from '../../charts/tooltip';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { nameReadMetadata, nameReadFailure } from '../../dashboard/nameReads.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt, noFactionMessage, useChosenFaction, useFactionDisplayNames } from '../../dashboard/factionChoice';
import type { NoFaction } from '../../dashboard/factionChoice';
import { headRead } from '../../dashboard/headProduct.ts';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { useHeadHour } from '../army-size/headHour.ts';
import { OPTIONS } from './definition';
import { eventHours, eventsThrough, memberNameHour, selectionName, upgradeLanes } from './upgradesData';
import type { LaneInfo } from './upgradesData';
import type { FactionRulesReply } from '../../generated/dashboard';
import { SPEC, axisAt } from '../../dashboard/regionTime.ts';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const NO_EVENTS: number[] = [];

export default function FactionUpgradesWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { groups, pick, tag: viewpointTag, hour: chosenAt } = useChosenFaction(widget, runtime, track, active);
  const displayNames = useFactionDisplayNames(track?.source);
  const faction = pick?.faction ?? null;
  const reply = useProduct<FactionRulesReply>(runtime.client, track && faction ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'faction-state',
    params: { view: 'rules', branchId: track.branch.id, faction: faction.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) } }) : null, active, undefined, track?.source);
  const data = reply.data;
  const following = widget.time.mode === 'follow';
  const { hourAt } = headRead({ widget, action: SPEC }, track, runtime.tracks);
  const events = useMemo(() => data ? eventHours(data) : NO_EVENTS, [data]);
  const clockHour = useHeadHour(runtime.clock, active && following && !!data, hour => { const at = hourAt(axisAt(hour)); return at === null ? 'outside' : String(eventsThrough(events, at)); });
  const at = hourAt(axisAt(following ? clockHour : runtime.clock.getHour())) ?? chosenAt;
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const shown = useShownHour();
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => following && active ? { read: () => { const head = runtime.clock.getSnapshot().position + axisOffset; shown(Math.floor(head)); return head; },
    subscribe: runtime.clock.subscribeFrame } : undefined, [following, active, runtime.clock, axisOffset, shown]);
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => (data?.members ?? []).flatMap(member => {
    const hour = memberNameHour(member); return hour === null || member.tag === null ? [] : [{ hour, tag: member.tag }]; }), [data]), active);
  const groupNames = useMemo(() => new Map(data?.groups.map(group => [group.group, group.name]) ?? []), [data]);
  const { lanes, info } = useMemo(() => data && at !== null ? upgradeLanes(data, at, following, {
    endedProgrammes: options.endedProgrammes !== 'hidden', groupName: group => groupNames.get(group) ?? 'Upgrade group name unavailable',
    country: (tag, hour) => tag === null ? { name: 'Country unavailable' } : nameReadMetadata(names.get(nameKey(hour, tag))) }) : { lanes: [], info: new Map<string, LaneInfo>() },
  [data, at, following, options.endedProgrammes, groupNames, names]);
  const unnamed = useMemo(() => {
    let count = 0;
    for (const lane of info.values()) for (const run of lane.runs) if (run.token !== null && !run.name) count++;
    return count;
  }, [info]);
  const diagnostics = useMemo(() => [...(data?.warnings ?? []), ...(unnamed ? [`${unnamed} faction selections have no name in the recorded content.`] : []),
    ...(reply.error ? [reply.error.message] : [])], [data, unnamed, reply.error]);
  useDiagnostics(onDiagnostics, diagnostics.length ? diagnostics : NO_WARNINGS);
  const menu = useContextMenu('Faction rules actions');
  const tooltip = useCallback((lane: TimelineLane, span: TimelineSpan | null, hour: number): TooltipContent | null => {
    const about = info.get(lane.id);
    if (!about) return null;
    const run = about.runs.find(item => item.firstHour <= hour && hour <= item.lastHour) ?? null;
    if (!span || !run) return { ...countryNameDetails(about), title: about.slot, subtitle: `${about.section} · ${dayLabel(hour)}`, rows: [{ label: 'Selection', value: 'Not recorded' }] };
    const index = about.runs.indexOf(run);
    return { ...countryNameDetails(about), title: selectionName(run), subtitle: `${about.section} · ${about.slot}`, rows: [
      { label: index === 0 || run.startsSegment ? 'Held from' : 'Selected by', value: dayLabel(run.firstHour) },
      { label: 'Held until', value: track && run.lastHour >= track.branch.lastHour ? 'Still held' : dayLabel(run.lastHour) },
      ...(run.reason ? [{ label: 'Name', value: 'Unavailable', tone: 'muted' as const }] : []),
    ] };
  }, [info, track]);
  const header = headerSlot && faction && at !== null && createPortal(<span className="pill">{factionNameAt(faction, at, displayNames[faction.id])}</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see faction rules." />;
  if (coverage.outside || at === null) return <WidgetState kind="outside" read={coverage} />;
  const groupFailure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!groups.data) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction rules…" /></>;
  if (!faction) return <>{groupFailure}<WidgetState kind="empty" icon="history" message={noFactionMessage(pick!.reason as NoFaction, following)} /></>;
  if (reply.error) return <>{groupFailure}{header}<WidgetState kind="error" failure={reply.error} onRetry={reply.retry} /></>;
  if (!data) return <>{groupFailure}{header}<WidgetState kind="loading" label="Loading faction rules…" /></>;
  if (!lanes.length) return <>{groupFailure}{header}<WidgetState kind="unavailable" message="This faction’s rules were not recorded." /></>;
  const window = resolveWindow(widget, track, runtime.tracks, following ? clockHour : runtime.clock.getHour());
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  const first = widget.time.mode === 'range' ? window.firstHour : track.branch.firstHour;
  const last = widget.time.mode === 'range' ? window.lastHour : track.branch.lastHour;
  const pinned = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = pinned ? at : null;
  const extent: [number, number] = [first, Math.max(first + 1, options.grow === true && playhead != null ? playhead : last + 1)];
  return <div className="timeline-scroll" data-detail={failureDetails([...names.values()].map(nameReadFailure))}>{header}{groupFailure}
    <Timeline key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, faction.id])} label={`${factionNameAt(faction, at, displayNames[faction.id])} rules and upgrades`}
      extent={extent} playhead={playhead} liveHead={liveHead} labelHour={following ? undefined : at} lanes={lanes} showIcons={false} showNames emptyLabel="Not recorded"
      preserveZoomOnExtentChange={options.grow === true} unobserved={data.unobserved} hoursPerSecond={hoursPerSecond}
      active={active} revision={`${lanes.length}:${at}`} tooltip={tooltip}
      description="Arrow keys step through lanes and bars."
      onContextMenu={(hour, x, y, _lane, span) => {
        const items = span ? [{ ...jumpItem(runtime, track.id, span.firstHour), label: `Jump to ${dayLabel(span.firstHour)}` },
          ...(span.lastHour < track.branch.lastHour ? [{ ...jumpItem(runtime, track.id, span.lastHour), label: `Jump to ${dayLabel(span.lastHour)}` }] : [])]
          : [jumpItem(runtime, track.id, Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, hour)))];
        menu.open(x, y, items);
      }} />
    {menu.menu}
  </div>;
}
