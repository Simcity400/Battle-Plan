import { useContext, useEffect, useId, useMemo, useRef } from 'react';
import type { CSSProperties, HTMLAttributes, KeyboardEvent, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import { Segmented } from '../../components/Segmented';
import ArtFallback from '../../components/ArtFallback';
import CountryComparisons from '../../dashboard/CountryComparisons';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import Timeline from '../../charts/timeline/Timeline';
import { PreparedKey } from '../../charts/PreparedKey';
import { PreparedLogCoverage } from '../../charts/log/LogList';
import { pendingAt } from '../../dashboard/preparedPrefix';
import type { PreparedCoverage } from '../../shell/preparation/contract';
import { fitTooltip, hideTooltip, showTooltip, tooltipText } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { HeaderActionsSlot } from '../../charts/ZoomReset';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProducts } from '../../dashboard/products';
import { useParityRows } from '../../dashboard/parityRows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { ProjectPresentation, SpecialProjectsReply } from '../../generated/dashboard';
import { OPTIONS, VIEWS } from './definition';
import { closedLane, currentProjectReply, currentProjects, projectPending, projectPrefix, earlierBound, historyCard, historyTooltip, iterationCount, observedAt, projectCountries, projectCountryHour, projectDate, projectHeading, projectHistory, projectHistoryExtent, projectTooltip, scientistName, validateSpecialProjectsReply } from './specialProjectsData';
import type { ProjectCard } from './specialProjectsData';
import '../national-focus/nationalFocus.css';
import './specialProjects.css';
import ProjectDetails from './ProjectDetails';

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

/** Current: running projects as the Research page's compact cards, then one-line
 * completed rows, the projects already complete when first recorded folded into
 * one row, and cancelled rows. At an hour preparation has not reached
 * (`preparing`), completed and cancelled projects come from the prefix and
 * current cards from the requested frame when it is available. */
function CurrentList({ reply, at, item, preparing = null, current = null, currentItem }: { reply: SpecialProjectsReply; at: number; item: ItemProps; preparing?: PreparedCoverage | null; current?: SpecialProjectsReply | null; currentItem?: ItemProps }) {
  const { running: held, completed, earlier, cancelled } = currentProjects(reply, at);
  const currentHour = current?.available[0]?.firstHour;
  const running = current && currentHour !== undefined ? currentProjects(current, currentHour).running : preparing ? [] : held;
  const runningReply = current ?? reply;
  const runningItem = currentItem ?? item;
  if (!reply.details && !preparing && !current && !running.length && !completed.length && !earlier.length && !cancelled.length) return <WidgetState kind="empty" message="No special projects yet." />;
  const row = (card: ProjectCard, line: string, className = '') => <div key={card.project.id} className={`focus-row special-project-row${className}`} {...item(card)}>
    <ProjectTile icon={card.info.icon} /><span className="focus-label"><strong>{card.info.name}</strong><span>{line}</span></span></div>;
  return <div className="special-project-list">
    {running.length ? <Section label="In progress" count={running.length}>{running.map(card => {
      const { run, info } = card;
      return <div key={card.project.id} className={`focus-now special-project-now${run.state === 'paused' ? ' is-paused' : run.state === 'unknown' ? ' is-unknown' : ''}`} {...runningItem(card)}>
        <ProjectTile icon={info.icon} />
        <span className="focus-now-text"><strong>{info.name}</strong><span>
          {run.state === 'paused' && <><b className="special-project-state is-paused">Paused</b>·</>}
          {run.state === 'unknown' && <><b className="special-project-state">State unknown</b>·</>}
          <Scientist reply={runningReply} info={info} /></span></span>
        <span className="focus-now-left"><strong>{run.progress}%</strong><small>{iterationCount(run.iterations)}</small></span>
        <span className="focus-progress" aria-hidden="true" style={{ '--focus-progress': Math.max(0, run.progress) / 100 } as CSSProperties}><span /></span>
      </div>;
    })}</Section> : <p className="special-project-none">{preparing && !current ? 'Projects in progress are still being prepared.' : 'No projects in progress.'}</p>}
    {completed.length + earlier.length > 0 && <Section label="Completed" count={completed.length + earlier.length}>
      {completed.map(card => {
        const person = reply.people[card.info.scientist].name;
        return row(card, `${dayLabel(card.run.completion!.hour)}${person ? ` · ${person}` : ''}`);
      })}
      {earlier.length > 0 && <details className="focus-details special-project-earlier">
        <summary className="focus-row special-project-row"><span className="blueprint-tile special-project-count">+{usNumber.format(earlier.length)}</span>
          <span className="focus-label"><strong>{`${usNumber.format(earlier.length)} completed by ${dayLabel(earlierBound(earlier))}`}</strong><span>Already complete when first recorded</span></span>
          <svg className="focus-chevron" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg></summary>
        <div className="special-project-earlier-grid">{earlier.map(card => <div key={card.project.id} className="special-project-earlier-item" {...item(card)}>
          <ProjectTile icon={card.info.icon} /><span>{card.info.name}</span></div>)}</div>
      </details>}
    </Section>}
    {cancelled.length > 0 && <Section label="Cancelled" count={cancelled.length}>
      {cancelled.map(card => row(card, `Cancelled · ${dayLabel(card.run.cancellation!.hour)} · ${card.run.progress}%`, ' is-cancelled'))}
    </Section>}
    <ProjectDetails details={reply.details} />
    {preparing && <PreparedLogCoverage coverage={preparing} />}
    {current && <p className="chart-line-key">Completed and cancelled history and start dates are still being prepared.</p>}
  </div>;
}

function ProjectPanel({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const parityRoot = useRef<HTMLDivElement>(null);
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const owner = useId();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'history' ? 'history' : 'live';
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = following && window ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const extent = projectHistoryExtent(first, last, playhead, options.grow === true);
  const at = Math.floor(playhead ?? last);
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => projectCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.firstHour ?? 0, track?.branch.lastHour ?? 0), [widget.countries, track, fallback, groups.data]);
  const tags = resolved.tags;
  const membershipUnknown = resolved.unobserved.some(span => span.firstHour <= at && at <= span.lastHour);
  const historyMembershipUnknown = resolved.unobserved.some(span => span.firstHour <= at && first <= span.lastHour);
  const requestedTags = view === 'live' ? tags.filter(tag => resolved.match.matches(tag, at)) : tags;
  const requests = track && resolved.ready ? requestedTags.map(tag => ({ sourceId: track.source.id, revision: track.source.revision, product: 'special-projects', params: { branchId: track.branch.id, tag, selectionTags: resolved.rootsFor.get(tag) ?? [], ...(view === 'live' ? { hour: at } : {}) } })) : [];
  const { data, error, retry } = useProducts<SpecialProjectsReply>(runtime.client, requests, active, validateSpecialProjectsReply);
  const parity = useMemo(() => data?.map(reply => ({ tag: reply.tag, identities: reply.identities,
    available: reply.available, readHours: reply.readHours, projects: reply.projects,
    presentations: reply.presentations, people: reply.people, iterationTexts: reply.iterationTexts,
    unavailable: reply.unavailable, warnings: reply.warnings, current: currentProjectReply(reply) })) ?? [], [data]);
  if (import.meta.env.MODE === 'parity') useParityRows(parityRoot, () => ({ kind: 'log', label: 'Special project values',
    complete: !!data && data.every(reply => projectPending(reply).length === 0), total: parity.length, rows: parity }));
  const wanted = data?.flatMap(reply => {
    const hour = projectCountryHour(reply, at, resolved.memberships.get(reply.tag));
    return hour === null ? [] : [{ tag: reply.tag, hour }];
  }) ?? [];
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && view === 'history' && tags.length > 1);
  const menu = useContextMenu('Project actions');
  const diagnostics = useMemo(() => [...(data?.flatMap(reply => [...reply.warnings, ...(currentProjectReply(reply)?.warnings ?? [])]) ?? []), ...(groups.data?.warnings ?? [])], [data, groups.data]);
  useDiagnostics(onDiagnostics, diagnostics);
  useEffect(() => () => hideTooltip(owner), [owner, view, at]);
  const from = extent[0];
  // Every country's reply reads one family's prefix: its hours still being prepared.
  const pending = useMemo(() => data?.map(projectPending).find(spans => spans.length), [data]);
  const lanes = useMemo(() => view !== 'history' ? [] : data?.flatMap((reply, country) => {
    const membership = resolved.memberships.get(reply.tag) ?? [];
    const history = projectHistory(reply, at, membership, from);
    const closing = closedLane(JSON.stringify([country, null, 'closed']), history);
    const projects = [...history.lanes.map(lane => ({ ...lane, id: JSON.stringify([country, lane.id]) })), ...(closing ? [closing] : [])];
    const hour = projectCountryHour(reply, at, membership);
    if (hour === null) return [];
    if (data.length === 1) return projects;
    return [projectHeading(JSON.stringify([country, null]), labelAt(hour)(reply.tag), reply.unavailable ? PROJECTS_UNRECORDED : (projects.length ? '' : 'No special projects yet.')), ...projects];
  }) ?? [], [data, at, from, labelAt, resolved.memberships, view]);
  const openMenu = (card: ProjectCard, x: number, y: number) => {
    if (!track) return;
    const start = jumpItem(runtime, track.id, card.project.start.hour);
    const completion = card.run.completion;
    menu.open(x, y, [{ ...start, label: `Jump to start · ${projectDate(card.project.start)}` },
      ...(completion ? [{ ...jumpItem(runtime, track.id, completion.hour), label: `Jump to completion · ${projectDate(completion)}` }] : [])]);
  };
  // Every listed project shares the hover tooltip, spoken description and Jump menu.
  const item = (reply: SpecialProjectsReply, preparingStart = false): ItemProps => card => {
    const tip = projectTooltip(reply, card, preparingStart);
    const show = (x: number, y: number) => showTooltip(owner, fitTooltip([tip], viewport), x, y);
    return { tabIndex: 0, role: 'group', 'aria-label': tooltipText(tip),
      onPointerMove: event => show(event.clientX, event.clientY), onPointerLeave: () => hideTooltip(owner),
      onFocus: event => { const box = event.currentTarget.getBoundingClientRect(); show(box.right, box.top); }, onBlur: () => hideTooltip(owner),
      onContextMenu: event => { event.preventDefault(); if (!preparingStart) openMenu(card, event.clientX, event.clientY); },
      onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
        if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey || event.key === 'Enter') {
          event.preventDefault(); if (!preparingStart) { const box = event.currentTarget.getBoundingClientRect(); openMenu(card, box.left, box.bottom); }
        }
      } };
  };
  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign and choose a country to see its special projects." />;
  else if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) body = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (at < track.branch.firstHour || at > track.branch.lastHour) body = <WidgetState kind="unavailable" message="This time is outside the recording." />;
  else if (error || groups.error) body = <WidgetState kind="error" detail={error ?? groups.error!} onRetry={error ? retry : groups.retry} />;
  else if (!groups.data) body = <WidgetState kind="loading" label="Loading countries…" />;
  else if (resolved.reason) body = <WidgetState kind="unavailable" message={resolved.reason} />;
  else if (view === 'live' && membershipUnknown) body = <WidgetState kind="unavailable" message="This group’s members were not recorded at this moment." />;
  else if (view === 'history' && !tags.length && historyMembershipUnknown) body = <WidgetState kind="unavailable" message="This group’s members were not recorded for part of this history." />;
  else if (!resolved.ready || !tags.length) body = <WidgetState kind="empty" message="No selected countries were recorded." />;
  else if (!data) body = <WidgetState kind="loading" label="Loading special projects…" />;
  else if (data.some(reply => !reply.details && currentProjectReply(reply)?.unavailable?.includes('still being prepared'))) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (data.every(reply => !reply.details && reply.unavailable && (!currentProjectReply(reply) || currentProjectReply(reply)!.unavailable))) body = <WidgetState kind="unavailable" message={PROJECTS_UNRECORDED} />;
  else if (view === 'live') {
    const reply = data.find(reply => resolved.match.matches(reply.tag, at));
    const prefix = reply ? projectPrefix(reply) : null;
    const current = reply ? currentProjectReply(reply) : null;
    if (!reply) body = <WidgetState kind="empty" message="No selected countries were recorded at this moment." />;
    // Past the prepared prefix: what it holds through its last hour, the rest still being prepared.
    else if (current && !observedAt(current, at)) body = <><WidgetState kind="unavailable" message="Projects were not recorded at this time." /><ProjectDetails details={reply.details} /></>;
    else if (current) body = <CurrentList reply={reply} at={prefix?.coverage.lastHour ?? at} item={item(reply)} preparing={prefix?.coverage} current={current} currentItem={item(current, true)} />;
    else if (prefix && pendingAt(prefix, at)) body = <CurrentList reply={reply} at={prefix.coverage.lastHour} item={item(reply)} preparing={prefix.coverage} />;
    else if (!observedAt(reply, at)) body = <><WidgetState kind="unavailable" message="Projects were not recorded at this time." /><ProjectDetails details={reply.details} /></>;
    else body = <CurrentList reply={reply} at={at} item={item(reply)} />;
  } else if (!pending?.length && !lanes.some(lane => lane.identity?.tile === 'blueprint' || lane.compact)) body = historyMembershipUnknown
    ? <WidgetState kind="unavailable" message="This group’s members were not recorded for part of this history." />
    : <WidgetState kind="empty" message="No special projects yet." />;
  else body = <><Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label="Special project history" extent={extent} playhead={playhead} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} showIcons={false} showNames={false} active={active} revision={lanes}
    barHeight={18} pending={pending}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
    labelMenu
    tooltip={(lane, span, hour) => {
      const [country, id] = JSON.parse(lane.id) as [number, string | null];
      return id === null ? null : historyTooltip(data[country], id, span, hour, at, resolved.memberships.get(data[country].tag));
    }}
    onContextMenu={(_hour, x, y, lane) => {
      if (!lane) return;
      const [country, id] = JSON.parse(lane.id) as [number, string | null];
      const reply = data[country];
      const card = id === null ? null : historyCard(reply, id, at, resolved.memberships.get(reply.tag));
      if (card) openMenu(card, x, y);
    }} /><PreparedKey pending={pending} /></>;
  return <div ref={parityRoot} className="special-projects">{body}{menu.menu}</div>;
}

/** A country's projects can be missing for several reasons; the card only says they are. */
const PROJECTS_UNRECORDED = 'Projects were not recorded.';

export default function SpecialProjectsWidget(props: WidgetProps) {
  const view = props.widget.options?.view === 'history' ? 'history' : 'live';
  usePlaybackHour(props.runtime.clock, props.active && props.widget.time.mode === 'follow');
  // The view switch sits beside Live, where the frame never clips it; a frame
  // without header actions (the catalogue preview) shows it beside the title.
  const actions = useContext(HeaderActionsSlot);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const groups = useCountryGroups(props.runtime, track, props.active);
  const fallback = defaultCountryTag(track);
  const selected = useMemo(() => projectCountries(props.widget.countries, fallback ? [fallback] : [], groups.data,
    track?.branch.firstHour ?? 0, track?.branch.lastHour ?? 0), [props.widget.countries, fallback, groups.data, track]);
  const window = track && resolveWindow(props.widget, track, props.runtime.tracks, props.runtime.clock.getHour());
  const at = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const tags = selected.tags.filter(tag => selected.match.matches(tag, at));
  const unknown = selected.unobserved.some(span => span.firstHour <= at && at <= span.lastHour);
  const tagsKey = JSON.stringify(tags);
  // Stable panels keep their scroll position while playback advances.
  const comparisonWidget = useMemo(() => ({ ...props.widget, countries: { kind: 'tags' as const, tags: JSON.parse(tagsKey) as string[] } }), [props.widget, tagsKey]);
  const slot = actions ?? props.headerSlot;
  const header = slot && createPortal(<Segmented value={view} options={VIEWS}
    onChange={value => props.setOption?.('view', value)} label="Special projects view" />, slot);
  const compare = view === 'live' && selected.ready && !unknown && tags.length > 1 && !groups.error;
  return <>{header}{compare
    ? <CountryComparisons {...props} widget={comparisonWidget} headerSlot={null} component={ProjectPanel} />
    // Each view gets its own panel, so a switch starts at the top instead of keeping the other view's scroll.
    : <ProjectPanel key={view} {...props} />}</>;
}
