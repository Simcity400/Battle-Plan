import { useEffect, useId, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import { Segmented } from '../../components/Segmented';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip, hideTooltip, showTooltip, tooltipText } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProducts } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { SpecialProjectsReply } from '../../generated/dashboard';
import IterationRow from './IterationRow';
import { OPTIONS, VIEWS } from './definition';
import { historyTooltip, liveCards, observedAt, projectDate, projectLanes, projectTooltip, scientistName, validateSpecialProjectsReply } from './specialProjectsData';
import type { ProjectCard } from './specialProjectsData';
import './specialProjects.css';

const NO_WARNINGS: string[] = [];

export default function SpecialProjectsWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const owner = useId();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'history' ? 'history' : 'live';
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = following && window ? window.lastHour : null;
  const extent: [number, number] = track ? widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour] : [0, 1];
  const at = Math.floor(playhead ?? extent[1]);
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, widget.countries.kind === 'all' ? track?.source.metadata.countries.map(country => country.tag) ?? [] : fallback ? [fallback] : [], groups.data, extent[1], 16), [widget.countries, track, fallback, groups.data, extent[1]]);
  const tags = resolved.tags;
  const membershipUnknown = resolved.membership !== null && resolved.unobserved.some(span => span.firstHour <= at && at <= span.lastHour);
  const requests = track ? tags.map(tag => ({ sourceId: track.source.id, revision: track.source.revision, product: 'special-projects', params: { branchId: track.branch.id, tag } })) : [];
  const { data, error, retry } = useProducts<SpecialProjectsReply>(runtime.client, requests, active, validateSpecialProjectsReply);
  const wanted = tags.map(tag => ({ tag, hour: at }));
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && view === 'history' && tags.length > 1);
  const menu = useContextMenu('Project actions');
  useEffect(() => {
    onDiagnostics(data?.flatMap(reply => reply.warnings) ?? NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  useEffect(() => () => hideTooltip(owner), [owner, view, at]);
  const lanes = useMemo(() => view !== 'history' ? [] : data?.flatMap((reply, country) => {
    const projects = projectLanes(reply, at, resolved.membership?.get(reply.tag)).map(lane => ({ ...lane, id: JSON.stringify([country, lane.id]) }));
    if (data.length === 1) return projects;
    const label = labelAt(at)(reply.tag);
    return [{ id: JSON.stringify([country, null]), label: reply.unavailable ?? (projects.length ? '' : 'No special projects yet.'), identity: { label: label.name, icon: label.flag }, spans: [] }, ...projects];
  }) ?? [], [data, at, labelAt, resolved.membership, view]);
  const header = headerSlot && createPortal(<><Segmented value={view} options={VIEWS} onChange={value => setOption?.('view', value)} label="Special projects view" />{view === 'live' && tags.length > 1 && <small className="special-project-country-note">First country shown</small>}</>, headerSlot);
  const openMenu = (card: ProjectCard, x: number, y: number) => {
    if (!track) return;
    const start = jumpItem(runtime, track.id, card.project.start.hour);
    const completion = card.run.completion;
    menu.open(x, y, [{ ...start, label: `Jump to start observation · ${projectDate(card.project.start)}` },
      ...(completion ? [{ ...jumpItem(runtime, track.id, completion.hour), label: `Jump to completion · ${projectDate(completion)}` }] : [])]);
  };
  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign and choose a country to see its special projects." />;
  else if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window || at < track.branch.firstHour || at > track.branch.lastHour) body = <WidgetState kind="unavailable" message="The chosen moment is not in this campaign." />;
  else if (error || groups.error && widget.countries.kind === 'group') body = <WidgetState kind="error" message={error ?? groups.error!} onRetry={error ? retry : groups.retry} />;
  else if (widget.countries.kind === 'group' && !groups.data) body = <WidgetState kind="loading" label="Loading country membership…" />;
  else if (resolved.reason) body = <WidgetState kind="unavailable" message={resolved.reason} />;
  else if (!data) body = <WidgetState kind="loading" label="Loading special projects…" />;
  else if (data.every(reply => reply.unavailable)) body = <WidgetState kind="unavailable" message={`Not in this recording. ${data[0].unavailable}`} />;
  else if (view === 'live') {
    const reply = data.find(reply => isMember(resolved.membership?.get(reply.tag), at));
    const cards = reply ? liveCards(reply, at, Number(options.completedRetentionDays)) : [];
    if (membershipUnknown) body = <WidgetState kind="unavailable" message="This group’s members were not recorded at this moment." />;
    else if (!reply) body = <WidgetState kind="empty" message="This group has no members at this moment." />;
    else if (!observedAt(reply, at)) body = <WidgetState kind="unavailable" message="Not in this recording. Projects were not recorded at this hour." />;
    else if (!cards.length) body = <WidgetState kind="empty" message="No projects running." />;
    else body = <div className="special-project-grid">{cards.map(card => {
      const { project, run, info } = card;
      const scientist = reply.people[info.scientist];
      const tip = projectTooltip(reply, card, at);
      const show = (x: number, y: number) => showTooltip(owner, fitTooltip([tip], viewport), x, y);
      return <div className={`special-project-card${run.state === 'completed' ? ' is-complete' : ''}`} key={project.id} tabIndex={0} role="group" aria-label={tooltipText(tip)}
        onPointerMove={event => show(event.clientX, event.clientY)} onPointerLeave={() => hideTooltip(owner)}
        onFocus={event => { const box = event.currentTarget.getBoundingClientRect(); show(box.right, box.top); }} onBlur={() => hideTooltip(owner)}
        onContextMenu={event => { event.preventDefault(); openMenu(card, event.clientX, event.clientY); }}
        onKeyDown={event => { if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey || event.key === 'Enter') { event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); openMenu(card, box.left, box.bottom); } }}>
        <div className="blueprint-tile">{info.icon ? <img src={info.icon} alt="" /> : <span aria-label={info.iconMissing ?? 'Project icon not recorded'}>?</span>}</div>
        <div className="special-project-detail"><b>{info.name}</b>
          <small>{`${info.specializationName ?? info.specialization ?? 'Unknown type'} project · `}{run.completion ? `completed ${projectDate(run.completion)}` : `${run.state === 'paused' ? 'paused · ' : run.state === 'unknown' ? 'state not recorded · ' : ''}${run.progress}%`}</small>
          <ProgressMeter value={run.progress / 100} state={run.state === 'completed' ? 'done' : run.state === 'running' ? 'live' : 'idle'} label="Project progress" />
          <IterationRow text={run.iterations === null ? null : reply.iterationTexts[run.iterations] ?? null} recorded={run.iterations !== null} />
          <div className="special-project-scientist">{scientist.portrait ? <img src={scientist.portrait} alt="" /> : <span className="special-project-portrait" aria-label={scientist.missing ?? 'Portrait not recorded'}>?</span>}<span>{scientistName(reply, info)}</span></div>
        </div>
      </div>;
    })}</div>;
  } else if (!lanes.some(lane => lane.identity?.tile === 'blueprint')) body = <WidgetState kind="empty" message="No special projects yet." />;
  else body = <Timeline label="Special project history" extent={extent} playhead={playhead} lanes={lanes} showIcons={false} showNames={false} active={active} revision={lanes}
    barHeight={18}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
    labelMenu
    tooltip={(lane, span, hour) => {
      const [country, id] = JSON.parse(lane.id) as [number, string | null];
      return id === null ? null : historyTooltip(data[country], id, span, hour, at);
    }}
    onContextMenu={(_hour, x, y, lane) => {
      if (!lane) return;
      const [country, id] = JSON.parse(lane.id) as [number, string | null];
      const reply = data[country];
      const project = reply.projects.find(project => project.id === id);
      const run = project?.runs.filter(run => run.firstHour <= at).at(-1);
      if (project && run) openMenu({ project, run, info: reply.presentations[run.presentation] }, x, y);
    }} />;
  return <div className="special-projects">{header}{resolved.dropped > 0 && <small>{resolved.dropped} countries exceed the 16-country comparison limit.</small>}{body}{menu.menu}</div>;
}
