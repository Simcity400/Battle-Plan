import { useGate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useId, useMemo, useRef } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Timeline from '../../charts/timeline/Timeline';
import { hideTooltip } from '../../charts/tooltip';
import { useIdentityHour } from '../../dashboard/atHead';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour';
import { motionOf } from '../../charts/easing.ts';
import { OPTIONS } from './definition';
import { closedLane, completions, historyPicture, historyProject, historyUnrecorded, historyTooltip, projectCountries, projectCountryHour, projectHeading, projectHistory,
  projectHistoryExtent } from './specialProjectsData';
import { useProjectHistory } from './useProjectHistory';
import './specialProjects.css';
import type { ProjectRecord, SpecialProjectsReply } from '../../generated/dashboard';

const PROJECTS_UNRECORDED = 'Projects were not recorded.';

export default function SpecialProjectsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const owner = useId();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget);
  const axisOffset = track?.axisOffset ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => projectCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.firstHour ?? 0, track?.branch.lastHour ?? 0),
    [widget.countries, track, fallback, groups.data]);
  const tags = resolved.tags;
  const historyParams = useMemo(() => track && resolved.ready ? tags.map(tag => ({ branchId: track.branch.id, country: countryFromKey(tag) })) : [],
    [track, resolved.ready, tags]);
  const membershipAt = useCallback((hour: number) => resolved.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour), [resolved.unobserved]);
  const structure = useCallback((clockHour: number) => {
    const hour = Math.floor(clockHour) + axisOffset;
    return String(membershipAt(hour));
  }, [axisOffset, membershipAt]);
  const playbackHour = useHeadHour(runtime.clock, active && following, structure);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const pinned = widget.time.mode === 'pin';
  const playhead = (following || pinned) && window ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const extent = projectHistoryExtent(first, last, playhead, options.grow === true);
  const from = extent[0];
  const openTo = following && track ? track.branch.lastHour : undefined;
  const picture = useCallback((replies: readonly SpecialProjectsReply[], hour: number) =>
    historyPicture(replies, hour, reply => resolved.memberships.get(reply.tag) ?? [], from, openTo), [resolved.memberships, from, openTo]);
  const historyRoot = useRef<HTMLDivElement>(null);
  const asked = Math.floor(playhead ?? last);
  const history = useProjectHistory(runtime, gate, track, historyParams, following, active, asked, picture, historyRoot);
  const data = history.replies;
  const { error, retry } = history;
  const at = following ? history.hour ?? asked : asked;
  const subscribe = useCallback((changed: () => void) => {
    if (!following) return () => {};
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [following, runtime.clock]);
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe } : undefined,
    [following, runtime.clock, axisOffset, subscribe]);
  const historyMembershipUnknown = resolved.unobserved.some(span => span.firstHour <= at && first <= span.lastHour);
  const identityHour = useIdentityHour(runtime.client, track, tags, runtime.clock, active && following && tags.length > 1,
    hour => Math.floor(hour) + axisOffset);
  const labelHour = following ? identityHour ?? at : at;
  const wanted = useMemo(() => data?.flatMap(reply => {
    const hour = projectCountryHour(reply, labelHour, resolved.memberships.get(reply.tag));
    return hour === null ? [] : [{ tag: reply.tag, hour }];
  }) ?? [], [data, labelHour, resolved.memberships]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && tags.length > 1);
  const menu = useContextMenu('Project actions');
  const notes = data;
  const baseWarnings = useMemo(() => [...(notes?.flatMap(reply => reply.warnings) ?? []), ...(groups.data?.warnings ?? [])], [notes, groups.data]);
  const detailWarnings = useRef<readonly string[]>([]);
  const reportWarnings = useCallback((warnings: readonly string[]) => {
    detailWarnings.current = warnings;
    onDiagnostics([...baseWarnings, ...warnings]);
  }, [onDiagnostics, baseWarnings]);
  useEffect(() => { reportWarnings(detailWarnings.current); }, [reportWarnings]);
  useEffect(() => () => onDiagnostics([]), [onDiagnostics]);
  useEffect(() => () => hideTooltip(owner), [owner, at]);
  const lanes = useMemo(() => notes?.flatMap((reply, country) => {
    const membership = resolved.memberships.get(reply.tag) ?? [];
    const drawn = projectHistory(reply, at, membership, from, openTo);
    const closing = closedLane(JSON.stringify([country, null, 'closed']), drawn);
    const projects = [...drawn.lanes.map(lane => ({ ...lane, id: JSON.stringify([country, lane.id]) })), ...(closing ? [closing] : [])];
    const hour = projectCountryHour(reply, labelHour, membership);
    if (projectCountryHour(reply, at, membership) === null || hour === null) return [];
    if (notes.length === 1) return projects;
    const empty = () => historyUnrecorded([reply], from, at, () => membership) ?? 'No special projects yet.';
    return [projectHeading(JSON.stringify([country, null]), labelAt(hour)(reply.tag), reply.unavailable ?? (projects.length ? '' : empty())), ...projects];
  }) ?? [], [notes, at, from, openTo, labelAt, labelHour, resolved.memberships]);
  useShownEachFrame(runtime.clock, data ? () => history.latest().through : null, history);
  const openMenu = (shown: ProjectRecord, x: number, y: number, hour: number) => {
    if (!track) return;
    const project = shown;
    const start = jumpItem(runtime, track.id, project.firstRead);
    menu.open(x, y, [{ ...start, label: `Jump to first recorded · ${dayLabel(project.firstRead)}` },
      ...completions(project, hour).map(entry => ({ ...jumpItem(runtime, track.id, entry.date), label: `Jump to completion · ${dayLabel(entry.date)}` }))]);
  };
  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign and choose a country to see its special projects." />;
  else if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) body = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (at < track.branch.firstHour || at > track.branch.lastHour) body = <WidgetState kind="outside" />;
  else if (error || (groups.error && !groups.data)) body = <WidgetState kind="error" failure={error ?? groups.error!} onRetry={error ? retry : groups.retry} />;
  else if (!groups.data) body = <WidgetState kind="loading" label="Loading countries…" />;
  else if (resolved.reason) body = <WidgetState kind="unavailable" message={resolved.reason} />;
  else if (!tags.length && historyMembershipUnknown) body = <WidgetState kind="unavailable" message="This group’s members were not recorded for part of this history." />;
  else if (!resolved.ready || !tags.length) body = <WidgetState kind="empty" message="No selected countries were recorded." />;
  else if (history.outside) body = <WidgetState kind="outside" />;
  else if (history.refused !== undefined) body = <WidgetState kind="unavailable" technical={history.refused} />;
  else if (!notes) body = <WidgetState kind="loading" label="Loading special projects…" />;
  else if (notes.every(reply => reply.unavailable)) body = <WidgetState kind="unavailable" message={notes.find(reply => reply.unavailable)?.unavailable ?? PROJECTS_UNRECORDED} />;
  else if (!lanes.some(lane => lane.identity?.tile === 'blueprint' || lane.compact)) {
    const unrecorded = historyUnrecorded(notes, from, at, reply => resolved.memberships.get(reply.tag));
    body = historyMembershipUnknown ? <WidgetState kind="unavailable" message="This group’s members were not recorded for part of this history." />
      : unrecorded ? <WidgetState kind="unavailable" message={unrecorded} /> : <WidgetState kind="empty" message="No special projects yet." />;
  }
  else body = <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label="Special project history" extent={extent} playhead={playhead}
    liveHead={liveHead} growTo={options.grow === true && following ? last + 1 : undefined} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} showIcons={false} showNames={false} active={active} revision={lanes}
    barHeight={18} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} labelMenu
    tooltip={(lane, span, hour) => {
      const [country, id] = JSON.parse(lane.id) as [number, string | null];
      const shown = history.latest(), reply = shown.replies[country], head = shown.through ?? at;
      return id === null || !reply ? null : historyTooltip(reply, id, span, hour, head, resolved.memberships.get(reply.tag));
    }}
    onContextMenu={(_hour, x, y, lane) => {
      if (!lane) return;
      const [country, id] = JSON.parse(lane.id) as [number, string | null];
      const shown = history.latest(), reply = shown.replies[country], head = shown.through ?? at;
      const project = id === null || !reply ? null : historyProject(reply, id, head, resolved.memberships.get(reply.tag));
      if (project) openMenu(project, x, y, head);
    }} />;
  return <div className="special-projects" ref={historyRoot}>{groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}{body}{menu.menu}</div>;
}
