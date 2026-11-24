import { useGate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { CSSProperties, HTMLAttributes, KeyboardEvent, MouseEvent, ReactNode } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import ArtFallback from '../../components/ArtFallback';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import WidgetState from '../../components/WidgetState';
import { worldFilter } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import { runAt } from '../../charts/series';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip, hideTooltip, showTooltip, tooltipText } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useIdentityHour } from '../../dashboard/atHead';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { openedOn } from '../../dashboard/gameWindows';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { PanelFeed } from '../../dashboard/panelFeed';
import { panelFeed } from '../../dashboard/panelFeed';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRecordedRows, useShownEachFrame } from '../../dashboard/widgetPerf';
import { perfLog } from '../../shell/perf';
import { useHeadHour } from '../army-size/headHour';
import { motionOf } from '../../charts/easing.ts';
import { OPTIONS } from './definition';
import { projectStructure, closedLane, currentKey, currentProjects, earlierBound, endedCount, historyPicture, historyProject, historyUnrecorded, historyTooltip, iterationMark, iterationTooltip,
  observedAt, completions, progressStyle, projectCountries, projectCountryHour, projectHeading, projectHistory, projectHistoryExtent, projectTooltip, readLabel, scientistName, shownCard, statusRead,
  unobservedReason, validateSpecialProjectsReply } from './specialProjectsData';
import type { ProjectCard } from './specialProjectsData';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import type { PlaybackController } from '../../dashboard/playback';
import ProjectDetails from './ProjectDetails';
import ProjectWindow from './ProjectWindow';
import { useProjectHistory } from './useProjectHistory';
import { Scientist } from './projectParts';
import './specialProjects.css';
import type { ProjectRecord, SpecialProjectsReply } from '../../generated/dashboard';

const PROJECTS_UNRECORDED = 'Projects were not recorded.';

const ProjectTile = ({ icon, name, reason, onClick }: { icon: string | null; name: string; reason?: string | null; onClick?: (event: MouseEvent<HTMLElement>) => void }) =>
  <span className={onClick ? 'blueprint-tile is-open' : 'blueprint-tile'} onClick={onClick}>{icon ? <img src={icon} alt="" /> : <ArtFallback icon="flask" name={name} reason={reason} />}</span>;

const Section = ({ label, count, children }: { label: string; count: number; children: ReactNode }) =>
  <><p className="special-project-section">{label}<span>{usNumber.format(count)}</span></p>{children}</>;

type ItemProps = (card: ProjectCard) => HTMLAttributes<HTMLElement>;

type HourNow = () => number;

function CurrentList({ reply, at, item, open, clock, feed, placement }: { clock: PlaybackController; reply: SpecialProjectsReply; at: number; item: ItemProps;
  open: (card: ProjectCard, event: MouseEvent<HTMLElement>) => void; placement?: number;
  feed: PanelFeed | null }) {
  const list = useRef<HTMLDivElement>(null);
  const valueId = useId();
  const exactRead = feed?.exactRead;
  const painted = !!feed?.pipeline && feed.pipe !== undefined && feed.follow !== undefined;
  useDrawnPanel(list, clock, feed, true);
  const groups = useMemo(() => currentProjects(reply, at), [reply, at]);
  const { running, completed, earlier } = groups;
  const rows = (g: typeof groups) => (['running', 'completed', 'earlier'] as const).flatMap(section =>
    g[section].map(card => JSON.stringify([section, card.project.id, card.run.presentation])));
  useRecordedRows('special-projects', perfLog && exactRead ? { hour: at, rows: () => rows(groups),
    exact: async () => rows(currentProjects(validateSpecialProjectsReply(await exactRead(at)), at)) } : undefined, placement);
  if (!running.length && !completed.length && !earlier.length) return <WidgetState kind="empty" message="No current projects." />;
  return <div className="special-project-list" ref={list}>
    <div className="special-project-rows">
      {running.length ? <Section label="In progress" count={running.length}>{running.map(card => {
        const { run, info } = card;
        const read = readLabel(statusRead(reply, card.project, at), at);
        const scientistRead = readLabel(run.scientistRead, at);
        return <div key={card.project.id} data-project={card.project.id} className={`special-project-now${run.state === 'paused' ? ' is-paused' : run.state === 'unknown' ? ' is-unknown' : ''}`}
          style={progressStyle(run.progress, painted) as CSSProperties | undefined} {...item(card)} aria-label={undefined}
          aria-labelledby={['name', 'progress', 'iterations'].map(part => `${valueId}-${card.project.id}-${part}`).join(' ')}>
          <ProjectTile icon={info.icon} name={info.name} reason={info.iconMissing} onClick={event => open(card, event)} />
          <strong className="special-project-name is-open" id={`${valueId}-${card.project.id}-name`} onClick={event => open(card, event)}>{info.name}</strong>
          <span className="special-project-who">
            {run.state === 'paused' && <b className="special-project-state is-paused">Paused</b>}
            {run.state === 'unknown' && <b className="special-project-state">State unknown</b>}
            <Scientist reply={reply} info={info} />{scientistRead && <small className="special-project-read">{scientistRead}</small>}</span>
          <span className="special-project-progress" aria-hidden="true"><span /></span>
          <strong className="special-project-percent" id={`${valueId}-${card.project.id}-progress`} data-paint="progress">{run.progress}%</strong>
          {read && <small className="special-project-read">{read}</small>}
          <small className="special-project-iterations" id={`${valueId}-${card.project.id}-iterations`} data-paint="iterations">{iterationMark(run.iterations)}</small>
        </div>;
      })}</Section> : <p data-panel-label="none" className="special-project-none">No projects in progress.</p>}
      {completed.length + earlier.length > 0 && <Section label="Completed" count={completed.length + earlier.length}>
        {completed.map(card => <div key={card.project.id} className="special-project-row special-project-done" {...item(card)}>
          <ProjectTile icon={card.info.icon} name={card.info.name} reason={card.info.iconMissing} onClick={event => open(card, event)} />
          <strong className="special-project-name is-open" data-panel-label={card.project.id} onClick={event => open(card, event)}>{card.info.name}</strong>
          <span className="special-project-who"><Scientist reply={reply} info={card.info} /></span>
          <span className="special-project-when">{readLabel(statusRead(reply, card.project, at), at) ?? 'Completed'}</span>
        </div>)}
        {earlier.length > 0 && <details className="special-project-earlier">
          <summary className="special-project-row"><span className="blueprint-tile special-project-count">+{usNumber.format(earlier.length)}</span>
            <strong className="special-project-name">{`${usNumber.format(earlier.length)} completed by ${dayLabel(earlierBound(earlier))}`}</strong>
            <span className="special-project-who">Already complete when first recorded</span>
            <svg className="special-project-chevron" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg></summary>
          <div className="special-project-earlier-grid">{earlier.map(card => <div key={card.project.id} className="special-project-earlier-item" {...item(card)}>
            <ProjectTile icon={card.info.icon} name={card.info.name} reason={card.info.iconMissing} onClick={event => open(card, event)} /><span className="is-open" onClick={event => open(card, event)}>{card.info.name}</span></div>)}</div>
        </details>}
      </Section>}
    </div>
    <table className="sr-only"><caption>Special projects</caption>
      <thead><tr><th scope="col">Project</th><th scope="col">Status</th><th scope="col">Scientist</th><th scope="col">Progress</th><th scope="col">Iterations</th></tr></thead>
      <tbody>{[...running, ...completed, ...earlier].map(card => {
        const active = card.run.state !== 'completed';
        return <tr key={card.project.id}><th scope="row">{card.info.name}</th><td>{card.run.state}</td>
          <td>{scientistName(reply, card.info)}</td>
          <td aria-labelledby={active ? `${valueId}-${card.project.id}-progress` : undefined}>{active ? null : `${card.run.progress}%`}</td>
          <td aria-labelledby={active ? `${valueId}-${card.project.id}-iterations` : undefined}>{active ? null : endedCount(card.run)}</td>
        </tr>;
      })}</tbody></table>
  </div>;
}

function ProjectPanel({ widget, runtime, active, onDiagnostics }: ComparisonProps) {
  const owner = useId();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'history' ? 'history' : 'live';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget);
  const axisOffset = track?.axisOffset ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => projectCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.firstHour ?? 0, track?.branch.lastHour ?? 0),
    [widget.countries, track, fallback, groups.data]);
  const tags = resolved.tags;
  const historyParams = useMemo(() => track && resolved.ready && view === 'history' ? tags.map(tag => ({ branchId: track.branch.id, country: countryFromKey(tag) })) : [],
    [track, resolved.ready, tags, view]);
  const target = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour;
  const tag = target === undefined ? undefined : tags.find(t => resolved.match.matches(t, target));
  const live = useHeadProduct(runtime, gate, 'special-projects', view === 'live' && tag ? { country: countryFromKey(tag) } : null,
    { active, validate: validateSpecialProjectsReply, structure: projectStructure });
  const currentData = useMemo(() => live.data ? [live.data] : undefined, [live.data]);
  const membershipAt = useCallback((hour: number) => resolved.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour), [resolved.unobserved]);
  const structure = useCallback((clockHour: number) => {
    const hour = Math.floor(clockHour) + axisOffset;
    if (view === 'live') return JSON.stringify([membershipAt(hour), tags.find(tag => resolved.match.matches(tag, hour)), resolved.match.known(hour)]);
    return String(membershipAt(hour));
  }, [view, axisOffset, resolved, membershipAt, tags]);
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
  const history = useProjectHistory(runtime, gate, track, historyParams, following, active && view === 'history', asked, picture, historyRoot);
  const data = view === 'history' ? history.replies : currentData;
  const { error, retry } = view === 'history' ? history : live;
  const at = view === 'live' ? live.at ?? target ?? last : following ? history.hour ?? asked : asked;
  const now = useCallback<HourNow>(() => following ? Math.floor(runtime.clock.getSnapshot().position) + axisOffset : at, [following, runtime.clock, axisOffset, at]);
  const subscribe = useCallback((changed: () => void) => {
    if (!following) return () => {};
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [following, runtime.clock]);
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe } : undefined,
    [following, runtime.clock, axisOffset, subscribe]);
  useShownEachFrame(runtime.clock, view === 'live' ? live.shownAt : null, live.live);
  const membershipUnknown = membershipAt(at);
  const historyMembershipUnknown = resolved.unobserved.some(span => span.firstHour <= at && first <= span.lastHour);
  const identityHour = useIdentityHour(runtime.client, track, tags, runtime.clock, active && following && view === 'history' && tags.length > 1,
    hour => Math.floor(hour) + axisOffset);
  const labelHour = following ? identityHour ?? at : at;
  const wanted = useMemo(() => data?.flatMap(reply => {
    const hour = projectCountryHour(reply, labelHour, resolved.memberships.get(reply.tag));
    return hour === null ? [] : [{ tag: reply.tag, hour }];
  }) ?? [], [data, labelHour, resolved.memberships]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && view === 'history' && tags.length > 1);
  const menu = useContextMenu('Project actions');
  const [opened, setOpened] = useState<{ on: string; id: string; x: number; y: number } | null>(null);
  const notes = data;
  const baseWarnings = useMemo(() => [...(notes?.flatMap(reply => reply.warnings) ?? []), ...(groups.data?.warnings ?? [])], [notes, groups.data]);
  const detailWarnings = useRef<readonly string[]>([]);
  const reportWarnings = useCallback((warnings: readonly string[]) => {
    detailWarnings.current = warnings;
    onDiagnostics([...baseWarnings, ...warnings]);
  }, [onDiagnostics, baseWarnings]);
  useEffect(() => { reportWarnings(detailWarnings.current); }, [reportWarnings]);
  useEffect(() => () => onDiagnostics([]), [onDiagnostics]);
  useEffect(() => () => hideTooltip(owner), [owner, view, at]);
  const lanes = useMemo(() => view !== 'history' ? [] : notes?.flatMap((reply, country) => {
    const membership = resolved.memberships.get(reply.tag) ?? [];
    const drawn = projectHistory(reply, at, membership, from, openTo);
    const closing = closedLane(JSON.stringify([country, null, 'closed']), drawn);
    const projects = [...drawn.lanes.map(lane => ({ ...lane, id: JSON.stringify([country, lane.id]) })), ...(closing ? [closing] : [])];
    const hour = projectCountryHour(reply, labelHour, membership);
    if (projectCountryHour(reply, at, membership) === null || hour === null) return [];
    if (notes.length === 1) return projects;
    const empty = () => historyUnrecorded([reply], from, at, () => membership) ?? 'No special projects yet.';
    return [projectHeading(JSON.stringify([country, null]), labelAt(hour)(reply.tag), reply.unavailable ?? (projects.length ? '' : empty())), ...projects];
  }) ?? [], [notes, at, from, openTo, labelAt, labelHour, resolved.memberships, view]);
  useShownEachFrame(runtime.clock, view === 'history' && data ? () => history.latest().through : null, history);
  const openMenu = (shown: ProjectRecord, x: number, y: number, hour: number) => {
    if (!track) return;
    const project = (view === 'live' ? (live.inspected()?.data ?? live.current())?.projects.find(project => project.id === shown.id) : undefined) ?? shown;
    const start = jumpItem(runtime, track.id, project.firstRead);
    menu.open(x, y, [{ ...start, label: `Jump to first recorded · ${dayLabel(project.firstRead)}` },
      ...completions(project, hour).map(entry => ({ ...jumpItem(runtime, track.id, entry.date), label: `Jump to completion · ${dayLabel(entry.date)}` }))]);
  };
  const item = (reply: SpecialProjectsReply): ItemProps => card => {
    const atNow = () => {
      const run = runAt(card.project.statuses, now());
      return run ? { ...card, run, info: reply.presentations[run.presentation] } : card;
    };
    const show = (x: number, y: number, target: EventTarget | null) => showTooltip(owner, fitTooltip([target instanceof Element && target.closest('.special-project-iterations')
      ? iterationTooltip(reply, atNow()) : projectTooltip(reply, atNow(), now())], viewport), x, y);
    return { tabIndex: 0, role: 'group', 'aria-label': tooltipText(projectTooltip(reply, card, at)),
      onPointerMove: event => show(event.clientX, event.clientY, event.target), onPointerLeave: () => hideTooltip(owner),
      onFocus: event => { const box = event.currentTarget.getBoundingClientRect(); show(box.right, box.top, null); }, onBlur: () => hideTooltip(owner),
      onContextMenu: event => { event.preventDefault(); openMenu(card.project, event.clientX, event.clientY, now()); },
      onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
        if (event.key === 'Enter') {
          event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); if (track) setOpened({ on: openedOn(track, reply.tag), id: card.project.id, x: box.left + 48, y: box.bottom });
        }
        else if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) {
          event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); openMenu(card.project, box.left, box.bottom, now());
        }
      } };
  };
  const noneAt = view === 'live' && widget.countries.kind === 'group' && !tags.length && groups.data && track
    && !resolved.match.known(track.branch.firstHour, track.branch.lastHour) ? worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, at, track.branch.lastHour) : null;
  const requestedNow = view === 'live' ? tags.filter(tag => resolved.match.matches(tag, at)) : tags;
  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign and choose a country to see its special projects." />;
  else if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) body = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (at < track.branch.firstHour || at > track.branch.lastHour) body = <WidgetState kind="outside" />;
  else if (error || (groups.error && !groups.data)) body = <WidgetState kind="error" failure={error ?? groups.error!} onRetry={error ? retry : groups.retry} />;
  else if (!groups.data) body = <WidgetState kind="loading" label="Loading countries…" />;
  else if (resolved.reason) body = <WidgetState kind="unavailable" message={resolved.reason} />;
  else if (view === 'live' && membershipUnknown) body = <WidgetState kind="unavailable" message="This group’s members were not recorded at this moment." />;
  else if (view === 'history' && !tags.length && historyMembershipUnknown) body = <WidgetState kind="unavailable" message="This group’s members were not recorded for part of this history." />;
  else if (noneAt?.reason) body = <NoMembersState reason={noneAt.reason} />;
  else if (!resolved.ready || !tags.length) body = <WidgetState kind="empty" message="No selected countries were recorded." />;
  else if (view === 'live' && !requestedNow.length) body = <WidgetState kind="empty" message="No selected countries were recorded at this moment." />;
  else if (view === 'history' && history.outside) body = <WidgetState kind="outside" />;
  else if (view === 'history' && history.refused !== undefined) body = <WidgetState kind="unavailable" technical={history.refused} />;
  else if (!notes) body = <WidgetState kind="loading" label="Loading special projects…" />;
  else if (notes.every(reply => reply.unavailable)) body = <WidgetState kind="unavailable" message={notes.find(reply => reply.unavailable)?.unavailable ?? PROJECTS_UNRECORDED} />;
  else if (view === 'live') {
    const reply = notes.find(reply => resolved.match.matches(reply.tag, at));
    if (!reply) body = <WidgetState kind="empty" message="No selected countries were recorded at this moment." />;
    else {
      const feed = panelFeed('special-projects', live, r => currentKey(r, at), widget, runtime);
      const observed = observedAt(reply, at);
      const details = <ProjectDetails runtime={runtime} widget={widget} tag={reply.tag} active={active} onWarnings={reportWarnings} />;
      const shown = opened?.on === openedOn(track, reply.tag) ? opened : null;
      const popup = shown && <ProjectWindow key={shown.id} runtime={runtime} widget={widget} tag={reply.tag} active={active} reply={reply} feed={feed}
        card={observed ? shownCard(reply, at, shown.id) : null} hour={at} through={live.shownAt} anchor={shown}
        missing={observed ? 'This project is not in progress or completed at this time.' : unobservedReason(reply, at)}
        onClose={() => setOpened(null)} />;
      body = observed ? <><CurrentList clock={runtime.clock} placement={live.placement} feed={feed} reply={reply} at={at} item={item(reply)}
        open={(card, event) => setOpened({ on: openedOn(track, reply.tag), id: card.project.id, x: event.clientX, y: event.clientY })} />{details}{popup}</>
        : <><WidgetState kind="unavailable" message={unobservedReason(reply, at)} />{details}{popup}</>;
    }
  } else if (!lanes.some(lane => lane.identity?.tile === 'blueprint' || lane.compact)) {
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
  return <div className="special-projects" ref={view === 'live' ? live.root : historyRoot}>{groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}{body}{menu.menu}</div>;
}

export default function SpecialProjectsWidget(props: WidgetProps) {
  const view = props.widget.options?.view === 'history' ? 'history' : 'live';
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const groups = useCountryGroups(props.runtime, track, props.active);
  const fallback = defaultCountryKey(track);
  const selected = useMemo(() => projectCountries(props.widget.countries, fallback ? [fallback] : [], groups.data,
    track?.branch.firstHour ?? 0, track?.branch.lastHour ?? 0), [props.widget.countries, fallback, groups.data, track]);
  const axisOffset = track?.axisOffset ?? 0;
  const chosenAt = useCallback((clockHour: number) => {
    const hour = Math.floor(clockHour) + axisOffset;
    return JSON.stringify([selected.tags.filter(tag => selected.match.matches(tag, hour)), selected.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour)]);
  }, [selected, axisOffset]);
  const playbackHour = useHeadHour(props.runtime.clock, props.active && props.widget.time.mode === 'follow', chosenAt);
  const window = track && resolveWindow(props.widget, track, props.runtime.tracks, playbackHour);
  const at = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const tags = selected.tags.filter(tag => selected.match.matches(tag, at));
  const unknown = selected.unobserved.some(span => span.firstHour <= at && at <= span.lastHour);
  const tagsKey = JSON.stringify(tags);
  const comparisonWidget = useMemo(() => ({ ...props.widget, countries: { kind: 'tags' as const, tags: JSON.parse(tagsKey) as string[] } }), [props.widget, tagsKey]);
  const compare = view === 'live' && selected.ready && !unknown && tags.length > 1 && !(groups.error && !groups.data);
  return compare
    ? <CountryComparisons {...props} widget={comparisonWidget} headerSlot={null} component={ProjectPanel} />
    : <ProjectPanel key={view} {...props} />;
}
