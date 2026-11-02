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
import { PreparedKey } from '../../charts/PreparedKey';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { nameReadMetadata, nameReadFailure } from '../../dashboard/nameReads.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt, noFactionMessage, useChosenFaction } from '../../dashboard/factionChoice';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS } from './definition';
import { memberNameHour, selectionName, upgradeLanes } from './upgradesData';
import type { LaneInfo } from './upgradesData';
import type { FactionRulesReply } from '../../generated/dashboard';

const NO_WARNINGS: string[] = [];

export default function FactionUpgradesWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const playhead = window && (widget.time.mode === 'follow' || widget.time.mode === 'pin') ? Math.floor(window.lastHour) : null;
  const at = Math.floor(playhead ?? window?.lastHour ?? track?.branch.lastHour ?? 0);
  const { groups, pick } = useChosenFaction(widget, runtime, track, at, active);
  const faction = pick?.faction ?? null;
  const reply = useProduct<FactionRulesReply>(runtime.client, track && faction ? { sourceId: track.source.id, revision: track.source.revision, product: 'faction-state',
    params: { view: 'rules', branchId: track.branch.id, faction: faction.id } } : null, active, undefined, track?.source);
  const data = reply.data;
  // Each member lane is named as its own holder was at its last recorded selection.
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => (data?.members ?? []).flatMap(member => {
    const hour = memberNameHour(member); return hour === null ? [] : [{ hour, tag: member.tag }]; }), [data]), active);
  const groupNames = useMemo(() => new Map(data?.groups.map(group => [group.group, group.name]) ?? []), [data]);
  const { lanes, info } = useMemo(() => data ? upgradeLanes(data, at, widget.time.mode === 'follow' ? at : null, {
    endedProgrammes: options.endedProgrammes !== 'hidden', groupName: group => groupNames.get(group) ?? 'Upgrade group name unavailable',
    country: (tag, hour) => nameReadMetadata(names.get(nameKey(hour, tag))) }) : { lanes: [], info: new Map<string, LaneInfo>() }, [data, at, widget.time.mode, options.endedProgrammes, groupNames, names]);
  const unnamed = useMemo(() => {
    let count = 0;
    for (const lane of info.values()) for (const run of lane.runs) if (run.token !== null && !run.name) count++;
    return count;
  }, [info]);
  const diagnostics = useMemo(() => [...(data?.warnings ?? []), ...(unnamed ? [`${unnamed} faction selections have no name in the recorded content.`] : []),
    ...(reply.error ? [reply.error] : [])], [data, unnamed, reply.error]);
  useDiagnostics(onDiagnostics, diagnostics.length ? diagnostics : NO_WARNINGS);
  const menu = useContextMenu('Faction rules actions');
  const tooltip = useCallback((lane: TimelineLane, span: TimelineSpan | null, hour: number): TooltipContent | null => {
    const about = info.get(lane.id);
    if (!about) return null;
    const run = about.runs.find(item => item.firstHour <= hour && hour <= item.lastHour) ?? null;
    if (!span || !run || run.token === null) return { ...countryNameDetails(about), title: about.slot, subtitle: `${about.section} · ${dayLabel(hour)}`, rows: [{ label: 'Selection', value: 'Not recorded' }] };
    const index = about.runs.indexOf(run);
    return { ...countryNameDetails(about), title: selectionName(run), subtitle: `${about.section} · ${about.slot}`, rows: [
      { label: index === 0 || run.startsSegment ? 'Held from' : 'Selected by', value: dayLabel(run.firstHour) },
      { label: 'Held until', value: track && run.lastHour >= track.branch.lastHour ? 'Still held' : dayLabel(run.lastHour) },
      ...(run.reason ? [{ label: 'Name', value: 'Unavailable', tone: 'muted' as const }] : []),
    ] };
  }, [info, track]);
  const header = headerSlot && faction && createPortal(<span className="pill">{factionNameAt(faction, at)}</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see faction rules." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  const groupFailure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!groups.data) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction rules…" /></>;
  if (!faction) return <>{groupFailure}<WidgetState kind="empty" icon="history" message={noFactionMessage(pick!.reason as 'none' | 'unobserved' | 'unknown', widget.time.mode === 'follow')} /></>;
  if (reply.error) return <>{groupFailure}{header}<WidgetState kind="error" failure={reply.error} onRetry={reply.retry} /></>;
  if (!data) return <>{groupFailure}{header}<WidgetState kind="loading" label="Loading faction rules…" /></>;
  if (!lanes.length) return <>{groupFailure}{header}<WidgetState kind="unavailable" message={data.pending.length ? 'This range is still being prepared.' : 'This faction’s rules were not recorded.'} /></>;
  const first = widget.time.mode === 'range' ? window.firstHour : track.branch.firstHour;
  const last = widget.time.mode === 'range' ? window.lastHour : track.branch.lastHour;
  const extent: [number, number] = [first, Math.max(first + 1, options.grow === true && playhead != null ? playhead : last + 1)];
  return <div className="timeline-scroll" data-detail={failureDetails([...names.values()].map(nameReadFailure))}>{header}{groupFailure}
    <Timeline key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, faction.id])} label={`${factionNameAt(faction, at)} rules and upgrades`}
      extent={extent} playhead={playhead} labelHour={at} lanes={lanes} showIcons={false} showNames emptyLabel="Not recorded"
      preserveZoomOnExtentChange={options.grow === true} unobserved={data.unobserved} pending={data.pending} hoursPerSecond={hoursPerSecond}
      active={active} revision={`${data.rules.length}:${lanes.length}:${at}`} tooltip={tooltip}
      description="Arrow keys step through lanes and bars."
      onContextMenu={(hour, x, y, _lane, span) => {
        const items = span ? [{ ...jumpItem(runtime, track.id, span.firstHour), label: `Jump to ${dayLabel(span.firstHour)}` },
          ...(span.lastHour < track.branch.lastHour ? [{ ...jumpItem(runtime, track.id, span.lastHour), label: `Jump to ${dayLabel(span.lastHour)}` }] : [])]
          : [jumpItem(runtime, track.id, Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, hour)))];
        menu.open(x, y, items);
      }} />
    <PreparedKey pending={data.pending} />
    {menu.menu}
  </div>;
}
