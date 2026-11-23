import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useContext, useEffect, useId, useLayoutEffect, useMemo, useRef } from 'react';
import type { CSSProperties, HTMLAttributes, KeyboardEvent, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import { Segmented } from '../../components/Segmented';
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
import { HeaderActionsSlot } from '../../charts/ZoomReset';
import { useIdentityHour } from '../../dashboard/atHead';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { PanelFeed } from '../../dashboard/panelFeed';
import { panelFeed } from '../../dashboard/panelFeed';
import { useProducts } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRecordedRows, useShownHour } from '../../dashboard/widgetPerf';
import { perfLog } from '../../shell/perf';
import { useHeadHour } from '../army-size/headHour';
import { motionOf } from '../../charts/easing.ts';
import { OPTIONS, VIEWS } from './definition';
import { projectStructure, closedLane, currentKey, currentProjects, earlierBound, historyCard, historyKey, historyTooltip, iterationCount, observedAt, projectCountries,
  projectCountryHour, projectDate, projectHeading, projectHistory, projectHistoryExtent, projectTooltip, scientistName,
  validateSpecialProjectsReply, progressStyle } from './specialProjectsData';
import type { ProjectCard } from './specialProjectsData';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import type { PlaybackController } from '../../dashboard/playback';
import ProjectDetails from './ProjectDetails';
import './specialProjects.css';
import type { ProjectPresentation, SpecialProjectsReply } from '../../generated/dashboard';

const PROJECTS_UNRECORDED = 'Projects were not recorded.';

const ProjectTile = ({ icon }: { icon: string | null }) =>
  <span className="blueprint-tile">{icon ? <img src={icon} alt="" /> : <ArtFallback icon="flask" />}</span>;

function Scientist({ reply, info }: { reply: SpecialProjectsReply; info: ProjectPresentation }) {
  const person = reply.people[info.scientist];
  if (!person.name) return <span className="special-project-scientist is-unassigned">{scientistName(reply, info)}</span>;
  return <span className="special-project-scientist">{person.portrait ? <img src={person.portrait} alt="" /> : <ArtFallback icon="helmet" />}<span>{person.name}</span></span>;
}

const Section = ({ label, count, children }: { label: string; count: number; children: ReactNode }) =>
  <><p className="special-project-section">{label}<span>{usNumber.format(count)}</span></p>{children}</>;

type ItemProps = (card: ProjectCard) => HTMLAttributes<HTMLElement>;

type HourNow = () => number;

function CurrentList({ reply, at, item, clock, feed, placement }: { clock: PlaybackController; reply: SpecialProjectsReply; at: number; item: ItemProps; placement?: number; feed: PanelFeed | null }) {
  const list = useRef<HTMLDivElement>(null);
  const valueId = useId();
  const widths = useMemo(() => {
    const progress = new Set<string>(), iterations = new Set<string>();
    for (const project of reply.projects) for (const run of project.runs) {
      if (run.state === 'completed' || run.state === 'cancelled') continue;
      progress.add((run.progress + '%').replace(/\d/g, '0'));
      iterations.add(iterationCount(run.iterations).replace(/\d/g, '0'));
    }
    return [...progress].map(text => <strong key={text}>{text}</strong>)
      .concat([...iterations].map(text => <small key={text}>{text}</small>));
  }, [reply]);
  const exactRead = feed?.exactRead;
  const painted = !!feed?.pipeline && feed.pipe !== undefined && feed.follow !== undefined;
  useDrawnPanel(list, clock, feed, true);
  const groups = useMemo(() => currentProjects(reply, at), [reply, at]);
  const { running, completed, earlier, cancelled } = groups;
  const rows = (g: typeof groups) => (['running', 'completed', 'earlier', 'cancelled'] as const).flatMap(section =>
    g[section].map(card => JSON.stringify([section, card.project.id, card.run.presentation])));
  useRecordedRows('special-projects', perfLog && exactRead ? { hour: at, rows: () => rows(groups),
    exact: async () => rows(currentProjects(validateSpecialProjectsReply(await exactRead(at)), at)) } : undefined, placement);
  if (!running.length && !completed.length && !earlier.length && !cancelled.length) return <WidgetState kind="empty" message="No special projects yet." />;
  const row = (card: ProjectCard, line: string, className = '') => <div key={card.project.id} className={`special-project-row${className}`} {...item(card)}>
    <ProjectTile icon={card.info.icon} /><span className="special-project-label"><strong data-panel-label={card.project.id}>{card.info.name}</strong><span>{line}</span></span></div>;
  return <div className="special-project-list" ref={list}>
    {running.length ? <Section label="In progress" count={running.length}>{running.map(card => {
      const { run, info } = card;
      return <div key={card.project.id} data-project={card.project.id} className={`special-project-now${run.state === 'paused' ? ' is-paused' : run.state === 'unknown' ? ' is-unknown' : ''}`}
        style={progressStyle(run.progress, painted) as CSSProperties | undefined} {...item(card)} aria-label={undefined}
        aria-labelledby={['name', 'progress', 'iterations'].map(part => `${valueId}-${card.project.id}-${part}`).join(' ')}>
        <ProjectTile icon={info.icon} />
        <span className="special-project-now-text"><strong id={`${valueId}-${card.project.id}-name`}>{info.name}</strong><span>
          {run.state === 'paused' && <><b className="special-project-state is-paused">Paused</b>·</>}
          {run.state === 'unknown' && <><b className="special-project-state">State unknown</b>·</>}
          <Scientist reply={reply} info={info} /></span></span>
        <span className="special-project-now-left"><span className="project-value-widths" aria-hidden="true">{widths}</span>
          <strong id={`${valueId}-${card.project.id}-progress`} data-paint="progress">{run.progress}%</strong><small id={`${valueId}-${card.project.id}-iterations`} data-paint="iterations">{iterationCount(run.iterations)}</small></span>
        <span className="special-project-progress" aria-hidden="true"><span /><i /></span>
      </div>;
    })}</Section> : <p data-panel-label="none" className="special-project-none">No projects in progress.</p>}
    {completed.length + earlier.length > 0 && <Section label="Completed" count={completed.length + earlier.length}>
      {completed.map(card => {
        const person = reply.people[card.info.scientist].name;
        return row(card, `${dayLabel(card.run.completion!.hour)}${person ? ` · ${person}` : ''}`);
      })}
      {earlier.length > 0 && <details className="special-project-earlier">
        <summary className="special-project-row"><span className="blueprint-tile special-project-count">+{usNumber.format(earlier.length)}</span>
          <span className="special-project-label"><strong>{`${usNumber.format(earlier.length)} completed by ${dayLabel(earlierBound(earlier))}`}</strong><span>Already complete when first recorded</span></span>
          <svg className="special-project-chevron" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg></summary>
        <div className="special-project-earlier-grid">{earlier.map(card => <div key={card.project.id} className="special-project-earlier-item" {...item(card)}>
          <ProjectTile icon={card.info.icon} /><span>{card.info.name}</span></div>)}</div>
      </details>}
    </Section>}
    {cancelled.length > 0 && <Section label="Cancelled" count={cancelled.length}>
      {cancelled.map(card => row(card, `Cancelled · ${dayLabel(card.run.cancellation!.hour)} · ${card.run.progress}%`, ' is-cancelled'))}
    </Section>}
    <table className="sr-only"><caption>Special projects</caption>
      <thead><tr><th scope="col">Project</th><th scope="col">Status</th><th scope="col">Scientist</th><th scope="col">Progress</th><th scope="col">Iterations</th></tr></thead>
      <tbody>{[...running, ...completed, ...earlier, ...cancelled].map(card => {
        const active = card.run.state !== 'completed' && card.run.state !== 'cancelled';
        return <tr key={card.project.id}><th scope="row">{card.info.name}</th><td>{card.run.state}</td>
          <td>{scientistName(reply, card.info)}</td>
          <td aria-labelledby={active ? `${valueId}-${card.project.id}-progress` : undefined}>{active ? null : `${card.run.progress}%`}</td>
          <td aria-labelledby={active ? `${valueId}-${card.project.id}-iterations` : undefined}>{active ? null : iterationCount(card.run.iterations)}</td>
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
  const axisOffset = track?.axisOffset ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => projectCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.firstHour ?? 0, track?.branch.lastHour ?? 0),
    [widget.countries, track, fallback, groups.data]);
  const tags = resolved.tags;
  const requests = useMemo(() => track && resolved.ready && view === 'history' ? tags.map(tag => ({ sourceId: track.source.id, revision: track.source.revision, product: 'special-projects',
    params: { branchId: track.branch.id, country: countryFromKey(tag) } })) : [], [track, resolved.ready, tags, view]);
  const history = useProducts<SpecialProjectsReply>(runtime.client, requests, active, validateSpecialProjectsReply, track?.source);
  const target = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour;
  const tag = target === undefined ? undefined : tags.find(t => resolved.match.matches(t, target));
  const live = useHeadProduct(runtime, widget, 'special-projects', view === 'live' && tag ? { country: countryFromKey(tag) } : null,
    { active, validate: validateSpecialProjectsReply, structure: projectStructure });
  const currentData = useMemo(() => live.data ? [live.data] : undefined, [live.data]);
  const data = view === 'history' ? history.data : currentData;
  const { error, retry } = view === 'history' ? history : live;
  const membershipAt = useCallback((hour: number) => resolved.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour), [resolved.unobserved]);
  const structure = useCallback((clockHour: number) => {
    const hour = Math.floor(clockHour) + axisOffset;
    if (view === 'live') return JSON.stringify([membershipAt(hour), tags.find(tag => resolved.match.matches(tag, hour)), resolved.match.known(hour)]);
    if (!data) return '';
    return `${membershipAt(hour)}\u0004${data.map(reply => historyKey(reply, hour, resolved.memberships.get(reply.tag))).join('\u0004')}`;
  }, [data, view, axisOffset, resolved, membershipAt, tags]);
  const playbackHour = useHeadHour(runtime.clock, active && following && (view === 'live' || !!data), structure);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const pinned = widget.time.mode === 'pin';
  const playhead = (following || pinned) && window ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const extent = projectHistoryExtent(first, last, playhead, options.grow === true);
  const at = view === 'live' ? live.at ?? target ?? last : Math.floor(playhead ?? last);
  const now = useCallback<HourNow>(() => following ? Math.floor(runtime.clock.getSnapshot().position) + axisOffset : at, [following, runtime.clock, axisOffset, at]);
  const subscribe = useCallback((changed: () => void) => {
    if (!following) return () => {};
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [following, runtime.clock]);
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe } : undefined,
    [following, runtime.clock, axisOffset, subscribe]);
  const shown = useShownHour();
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
  const bounded = useProducts<SpecialProjectsReply>(runtime.client, requests.map(r => ({ ...r, params: { ...r.params, hour: at } })), active && view === 'history', validateSpecialProjectsReply, track?.source);
  const notes = view === 'history' ? bounded.data : currentData;
  const baseWarnings = useMemo(() => [...(notes?.flatMap(reply => reply.warnings) ?? []), ...(groups.data?.warnings ?? [])], [notes, groups.data]);
  const detailWarnings = useRef<readonly string[]>([]);
  const reportWarnings = useCallback((warnings: readonly string[]) => {
    detailWarnings.current = warnings;
    onDiagnostics([...baseWarnings, ...warnings]);
  }, [onDiagnostics, baseWarnings]);
  useEffect(() => { reportWarnings(detailWarnings.current); }, [reportWarnings]);
  useEffect(() => () => onDiagnostics([]), [onDiagnostics]);
  useEffect(() => () => hideTooltip(owner), [owner, view, at]);
  const from = extent[0];
  const lanes = useMemo(() => view !== 'history' ? [] : data?.flatMap((reply, country) => {
    const membership = resolved.memberships.get(reply.tag) ?? [];
    const history = projectHistory(reply, at, membership, from, following);
    const closing = closedLane(JSON.stringify([country, null, 'closed']), history);
    const projects = [...history.lanes.map(lane => ({ ...lane, id: JSON.stringify([country, lane.id]) })), ...(closing ? [closing] : [])];
    const hour = projectCountryHour(reply, labelHour, membership);
    if (projectCountryHour(reply, at, membership) === null || hour === null) return [];
    if (data.length === 1) return projects;
    return [projectHeading(JSON.stringify([country, null]), labelAt(hour)(reply.tag), reply.unavailable ? PROJECTS_UNRECORDED : (projects.length ? '' : 'No special projects yet.')), ...projects];
  }) ?? [], [data, at, from, following, labelAt, labelHour, resolved.memberships, view]);
  useLayoutEffect(() => {
    if (view !== 'history' || !data) return;
    const report = () => shown(now());
    report();
    return subscribe(report);
  }, [view, data, now, subscribe, shown]);
  const openMenu = (card: ProjectCard, x: number, y: number) => {
    if (!track) return;
    const start = jumpItem(runtime, track.id, card.project.start.hour);
    const completion = card.run.completion;
    menu.open(x, y, [{ ...start, label: `Jump to start · ${projectDate(card.project.start)}` },
      ...(completion ? [{ ...jumpItem(runtime, track.id, completion.hour), label: `Jump to completion · ${projectDate(completion)}` }] : [])]);
  };
  const item = (reply: SpecialProjectsReply): ItemProps => card => {
    const atNow = () => {
      const run = runAt(card.project.runs, now());
      return run ? { ...card, run, info: reply.presentations[run.presentation] } : card;
    };
    const show = (x: number, y: number) => showTooltip(owner, fitTooltip([projectTooltip(reply, atNow())], viewport), x, y);
    return { tabIndex: 0, role: 'group', 'aria-label': tooltipText(projectTooltip(reply, card)),
      onPointerMove: event => show(event.clientX, event.clientY), onPointerLeave: () => hideTooltip(owner),
      onFocus: event => { const box = event.currentTarget.getBoundingClientRect(); show(box.right, box.top); }, onBlur: () => hideTooltip(owner),
      onContextMenu: event => { event.preventDefault(); openMenu(atNow(), event.clientX, event.clientY); },
      onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
        if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey || event.key === 'Enter') {
          event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); openMenu(atNow(), box.left, box.bottom);
        }
      } };
  };
  const noneAt = view === 'live' && widget.countries.kind === 'group' && !tags.length && groups.data && track
    && !resolved.match.known(track.branch.firstHour, track.branch.lastHour) ? worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, at, track.branch.lastHour) : null;
  const requestedNow = view === 'live' ? tags.filter(tag => resolved.match.matches(tag, at)) : tags;
  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign and choose a country to see its special projects." />;
  else if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) body = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (at < track.branch.firstHour || at > track.branch.lastHour) body = <WidgetState kind="unavailable" message="This time is outside the recording." />;
  else if (error || (groups.error && !groups.data)) body = <WidgetState kind="error" failure={error ?? groups.error!} onRetry={error ? retry : groups.retry} />;
  else if (!groups.data) body = <WidgetState kind="loading" label="Loading countries…" />;
  else if (resolved.reason) body = <WidgetState kind="unavailable" message={resolved.reason} />;
  else if (view === 'live' && membershipUnknown) body = <WidgetState kind="unavailable" message="This group’s members were not recorded at this moment." />;
  else if (view === 'history' && !tags.length && historyMembershipUnknown) body = <WidgetState kind="unavailable" message="This group’s members were not recorded for part of this history." />;
  else if (noneAt?.reason) body = <NoMembersState reason={noneAt.reason} />;
  else if (!resolved.ready || !tags.length) body = <WidgetState kind="empty" message="No selected countries were recorded." />;
  else if (view === 'live' && !requestedNow.length) body = <WidgetState kind="empty" message="No selected countries were recorded at this moment." />;
  else if (!data) body = <WidgetState kind="loading" label="Loading special projects…" />;
  else if (data.every(reply => reply.unavailable)) body = <WidgetState kind="unavailable" message={data.find(reply => reply.unavailable)?.unavailable ?? PROJECTS_UNRECORDED} />;
  else if (view === 'live') {
    const reply = data.find(reply => resolved.match.matches(reply.tag, at));
    if (!reply) body = <WidgetState kind="empty" message="No selected countries were recorded at this moment." />;
    else {
      const details = <ProjectDetails runtime={runtime} widget={widget} tag={reply.tag} active={active} onWarnings={reportWarnings} />;
      body = observedAt(reply, at) ? <><CurrentList clock={runtime.clock} placement={live.placement} feed={panelFeed('special-projects', live, r => currentKey(r, at), widget, runtime)}
        reply={reply} at={at} item={item(reply)} />{details}</>
        : <><WidgetState kind="unavailable" message="Projects were not recorded at this time." />{details}</>;
    }
  } else if (!lanes.some(lane => lane.identity?.tile === 'blueprint' || lane.compact)) body = historyMembershipUnknown
    ? <WidgetState kind="unavailable" message="This group’s members were not recorded for part of this history." />
    : <WidgetState kind="empty" message="No special projects yet." />;
  else body = <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label="Special project history" extent={extent} playhead={playhead}
    liveHead={liveHead} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} showIcons={false} showNames={false} active={active} revision={lanes}
    barHeight={18} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} labelMenu
    tooltip={(lane, span, hour) => {
      const [country, id] = JSON.parse(lane.id) as [number, string | null];
      return id === null ? null : historyTooltip(data[country], id, span, hour, now(), resolved.memberships.get(data[country].tag));
    }}
    onContextMenu={(_hour, x, y, lane) => {
      if (!lane) return;
      const [country, id] = JSON.parse(lane.id) as [number, string | null];
      const reply = data[country];
      const card = id === null ? null : historyCard(reply, id, now(), resolved.memberships.get(reply.tag));
      if (card) openMenu(card, x, y);
    }} />;
  return <div className="special-projects" ref={view === 'live' ? live.root : undefined}>{groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}{body}{menu.menu}</div>;
}

export default function SpecialProjectsWidget(props: WidgetProps) {
  const view = props.widget.options?.view === 'history' ? 'history' : 'live';
  const actions = useContext(HeaderActionsSlot);
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
  const slot = actions ?? props.headerSlot;
  const header = slot && createPortal(<Segmented value={view} options={VIEWS}
    onChange={value => props.setOption?.('view', value)} label="Special projects view" />, slot);
  const compare = view === 'live' && selected.ready && !unknown && tags.length > 1 && !(groups.error && !groups.data);
  return <>{header}{compare
    ? <CountryComparisons {...props} widget={comparisonWidget} headerSlot={null} component={ProjectPanel} />
    : <ProjectPanel key={view} {...props} />}</>;
}
