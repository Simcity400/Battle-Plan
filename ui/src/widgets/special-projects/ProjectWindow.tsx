import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import ArtFallback from '../../components/ArtFallback';
import FloatingPanel from '../../components/FloatingPanel';
import type { FloatingAnchor } from '../../components/FloatingPanel';
import WidgetState from '../../components/WidgetState';
import '../../components/Dossier.css';
import { dayLabel } from '../../charts/draw';
import { formatDateTime } from '../../dashboard/calendar';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { BoundTrack } from '../../dashboard/model';
import type { PanelFeed } from '../../dashboard/panelFeed';
import { panelFeed } from '../../dashboard/panelFeed';
import { useProduct } from '../../dashboard/products';
import { OpenedLink } from '../research/TechnologyWindow';
import GameText, { GameArtFor, GameBlocks, GameIcons, GameRuns, type EntityLink } from '../research/GameText';
import { trimLabel, trimValue } from '../research/gameRuns';
import { DetailValues, Name, Painted } from './ProjectDetails';
import type { Asked } from './ProjectDetails';
import { REWARDS_PANEL, detailTexts, effectsRead, effectsStructure, optionEffects, rewardsStructure, validateProjectDetails, validateProjectEffects, validateProjectIteration } from './projectDetailsData';
import { episodeBound, iterationCount, joinDue, lastScientist, readLabel, runtimeProject, scientistName, statusRead } from './specialProjectsData';
import type { ProjectCard, RuntimeMatch } from './specialProjectsData';
import type { ProjectChoice, ProjectEffects, ProjectRuntimeDetails, SpecialProjectDetails, SpecialProjectEffects, SpecialProjectIteration, ProjectStatus, SpecialProjectsReply } from '../../generated/dashboard';

const STATES = { running: 'In progress', paused: 'Paused', unknown: 'State unknown', completed: 'Completed', cancelled: 'Cancelled' } as const;
const STATE_TONES = { running: '', paused: ' is-paused', unknown: ' is-quiet', completed: ' is-done', cancelled: ' is-quiet' } as const;

type WindowProps = Asked & { reply: SpecialProjectsReply; card: ProjectCard | null; hour: number; through: () => number | null; missing: string; feed: PanelFeed | null;
  anchor: FloatingAnchor; onClose: () => void };

export default function ProjectWindow({ reply, card, hour, through, missing, feed, anchor, onClose, ...asked }: WindowProps) {
  return <FloatingPanel title="Special project" label="special project details" anchor={anchor} onClose={onClose} className="project-window">
    <GameArtFor runtime={asked.runtime} widget={asked.widget} active={asked.active}>{card ? <div className="dossier is-blueprint">
      <Summary reply={reply} card={card} hour={hour} feed={feed} active={asked.active} clock={asked.runtime.clock} />
      <Runtime {...asked} reply={reply} card={card} hour={hour} through={through} />
    </div> : <div className="dossier-body"><WidgetState kind="unavailable" message={missing} /></div>}</GameArtFor>
  </FloatingPanel>;
}

function Summary({ reply, card, hour, feed, active, clock }: { reply: SpecialProjectsReply; card: ProjectCard; hour: number; feed: PanelFeed | null; active: boolean; clock: Asked['runtime']['clock'] }) {
  const facts = useRef<HTMLDivElement>(null);
  const { project, run, info } = card;
  const done = run.state === 'completed';
  useDrawnPanel(facts, clock, done ? null : feed, active);
  const person = reply.people[info.scientist];
  const assigned = lastScientist(reply, info, run);
  const scientistRead = readLabel(run.scientistRead, hour);
  const read = readLabel(statusRead(reply, project, hour), hour);
  return <>
    <header className="dossier-hero">
      <span className="dossier-art is-blueprint">{info.icon ? <img src={info.icon} alt="" /> : <ArtFallback icon="flask" />}</span>
      <div className="dossier-title">
        <h3>{info.name}</h3>
        <div className="dossier-chips">
          {(run.state === 'paused' || run.state === 'unknown') && <span className={`dossier-chip is-state${STATE_TONES[run.state]}`}>{STATES[run.state]}</span>}
          <span className="dossier-chip" title={assigned ?? undefined}>{person.name && (person.portrait ? <img src={person.portrait} alt="" /> : <ArtFallback icon="helmet" />)}
            <span>{scientistName(reply, info)}</span>{scientistRead && <small>{scientistRead}</small>}</span>
        </div>
        {read && !done && <p className="dossier-note">{`Status ${read}`}</p>}
      </div>
    </header>
    <div className={`dossier-progress${done ? ' is-done' : ''}`} ref={facts} data-project={done ? undefined : project.id}
      style={{ '--project-progress': Math.max(0, run.progress) / 100 } as CSSProperties}>
      {done ? <strong className="dossier-progress-value">{read ? `Completed · ${read}` : 'Completed'}</strong> : <>
        <strong className="dossier-progress-value" data-paint="progress">{run.progress}%</strong>
        <span className="special-project-progress" aria-hidden="true"><span /></span>
      </>}
    </div>
  </>;
}

function whenDue(hour: number, head: () => number, subscribe: (fn: () => void) => () => void, wake: () => void) {
  const frame = () => { if (head() >= hour) { stop(); wake(); } };
  const stop = subscribe(frame);
  frame();
  return stop;
}

function Runtime({ runtime, widget, tag, active, reply, card, hour, through }: Asked & {
  reply: SpecialProjectsReply; card: ProjectCard; hour: number; through: () => number | null;
}) {
  const box = useRef<HTMLDivElement | null>(null);
  const head = useHeadProduct<SpecialProjectDetails>(runtime, widget, 'special-project-details', { country: countryFromKey(tag) },
    { active, validate: validateProjectDetails, structure: rewardsStructure, panel: REWARDS_PANEL });
  const { data, error, retry, root } = head;
  useDrawnPanel(box, runtime.clock, panelFeed(REWARDS_PANEL, head, rewardsStructure, widget, runtime), active);
  const values = useMemo(() => data ? detailTexts(data) : new Map<string, string>(), [data]);
  const ref = useCallback((element: HTMLDivElement | null) => { box.current = element; root(element); }, [root]);
  const listed = useCallback(() => through() ?? hour, [hour, through]);
  const current = useCallback(() => Math.max(hour, listed()), [hour, listed]);
  const bound = episodeBound(reply, card.project, through());
  const due = data ? joinDue(data, card, current(), bound) : null;
  const [, wake] = useState(0);
  useEffect(() => {
    if (due) return whenDue(due.hour, due.list ? listed : current, runtime.clock.subscribeFrame, () => wake(n => n + 1));
  }, [due?.hour, due?.list, listed, current, runtime.clock]);
  const match = data ? runtimeProject(data, card, current(), bound) : null;
  return <div className="dossier-body" ref={ref}>
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : !data || !match ? <WidgetState kind="loading" label="Loading project rewards…" />
      : typeof match === 'string' ? <p className="dossier-missing">{match}</p>
        : <DetailValues.Provider value={values}><Rewards runtime={runtime} widget={widget} tag={tag} active={active} hour={data.hour} match={match} run={card.run} /></DetailValues.Provider>}
  </div>;
}

function Output({ value, state, onOpen }: { value: ProjectEffects | null | undefined; state: string | null; onOpen: (link: EntityLink) => void }) {
  if (state) return <p className="dossier-missing">{state}</p>;
  if (!value) return <p className="dossier-missing">Effects unavailable.</p>;
  const refusals = Object.entries(value.refusals);
  return <>
    {value.runs.length > 0 ? <GameBlocks runs={value.runs} onOpen={onOpen} /> : !refusals.length && <p className="dossier-note">No effects to describe.</p>}
    {refusals.map(([reason, count]) => <p className="dossier-missing" key={reason}>Not recorded: {reason}{(count ?? 0) > 1 ? ` (${count})` : ''}</p>)}
  </>;
}

function Rewards({ runtime, widget, tag, active, hour, match, run }: Asked & { hour: number; match: RuntimeMatch; run: ProjectStatus }) {
  const { project } = match;
  const done = run.state === 'completed';
  const box = useRef<HTMLDivElement>(null);
  const effects = useHeadProduct<SpecialProjectEffects>(runtime, widget, 'special-project-effects', { country: countryFromKey(tag), owner: project.owner, project: project.id },
    { active, validate: validateProjectEffects, structure: effectsStructure });
  const read = effects.error ? effects.error.message : !effects.data ? 'Loading effects…' : effectsRead(hour, project, effects.data);
  const state = read ?? (effects.data?.output ? null : effects.data?.unavailable ?? null);
  const options = read ?? (effects.data ? optionEffects(project, effects.data) : 'Loading effects…');
  useLayoutEffect(() => { effects.root(box.current); }, [effects.root]);
  const [opened, setOpened] = useState<EntityLink | null>(null);
  const at = `j:${project.owner}:${project.id}`;
  return <div ref={box} className="project-window-sections">
    {!done && <section className="dossier-section">
      <h4>Pending reward</h4>
      {!project.pendingKnown ? <p className="dossier-missing">Pending reward not recorded.</p> : !project.pending ? <p className="dossier-note">No reward choice pending.</p> : <>
        <p className="project-page-reward"><Name label={project.pending.label} /></p>
        {project.pending.description && <p className="dossier-note">{project.pending.description}</p>}
        {project.pending.unavailable && <p className="dossier-missing">{project.pending.unavailable}</p>}
        <div className="project-page-options">{project.pending.options.map((option, index) => <div className="project-page-option" key={index}>
          <h5><Name label={option.label} /></h5><Output value={typeof options === 'string' ? null : options[index]} state={typeof options === 'string' ? options : null} onOpen={setOpened} /></div>)}</div>
      </>}
    </section>}
    <Iterations runtime={runtime} widget={widget} tag={tag} active={active} project={project} choices={match.choices} run={run} onOpen={setOpened} />
    <section className="dossier-section">
      <h4>Project output</h4>
      <Output value={effects.data?.output} state={state} onOpen={setOpened} />
    </section>
    <details className="dossier-section project-page-variables">
      <summary>Project variables <Painted at={`${at}:read`} /></summary>
      {project.variables === null ? <p className="dossier-missing">Variables not recorded.</p> : project.variables.length === 0 ? <p className="dossier-note">No project variables.</p>
        : <dl>{project.variables.map((variable, index) => <div key={index}><dt>{variable.name}</dt><dd><Painted at={`${at}:var:${index}`} /></dd></div>)}</dl>}
      {project.seed !== null && <p className="dossier-note">Variable random seed <Painted at={`${at}:seed`} /></p>}
    </details>
    <OpenedLink runtime={runtime} widget={widget} tag={tag} active={active} opened={opened} onClose={() => setOpened(null)} />
  </div>;
}

function Iterations({ runtime, widget, tag, active, project, choices, run, onOpen }: Asked & { project: ProjectRuntimeDetails; choices: RuntimeMatch['choices']; run: ProjectStatus;
  onOpen: (link: EntityLink) => void }) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const listed = (shown: readonly ProjectChoice[]) => <ol className="project-window-timeline">{shown.map(choice => <li key={choice.ordinal}>
    <time>{dayLabel(choice.date)}</time>
    <p className="project-window-choice-name"><Name label={choice.reward} /><span className="project-window-option"> · <Name label={choice.option} /></span></p>
    <IterationOutput runtime={runtime} track={track} tag={tag} active={active} project={project} choice={choice} onOpen={onOpen} />
  </li>)}</ol>;
  const current = Array.isArray(choices) ? choices.filter(choice => !choice.earlier) : [];
  const earlier = Array.isArray(choices) ? choices.filter(choice => choice.earlier) : [];
  const read = project.historyReadHour === null ? '' : ` · read ${formatDateTime(project.historyReadHour)}`;
  return <section className="dossier-section">
    <h4>Iterations<span className="project-window-count">{`${iterationCount(run.iterations)}${read}`}</span></h4>
    {choices === null ? <p className="dossier-missing">Iterations not recorded.</p> : typeof choices === 'string' ? <p className="dossier-missing">{choices}</p>
      : current.length === 0 ? <p className="dossier-note">No iterations listed in this attempt.</p> : listed(current)}
    {earlier.length > 0 && <details className="project-window-earlier"><summary>{`Earlier attempts · ${earlier.length} listed`}</summary>{listed(earlier)}</details>}
  </section>;
}

function IterationOutput({ runtime, track, tag, active, project, choice, onOpen }: Pick<Asked, 'runtime' | 'tag' | 'active'> & {
  track: BoundTrack | undefined; project: ProjectRuntimeDetails; choice: ProjectChoice; onOpen: (link: EntityLink) => void;
}) {
  const recorded = !!track && choice.date >= track.branch.firstHour && choice.date <= track.branch.lastHour;
  const read = project.historyReadHour;
  const request = useMemo(() => track && recorded && read !== null ? { sourceId: track.source.id, revision: track.source.revision, product: 'special-project-iteration',
    params: { branchId: track.branch.id, hour: choice.date, country: countryFromKey(tag), owner: project.owner, project: project.id, ordinal: choice.ordinal, read } } : null,
  [track, recorded, read, choice.date, choice.ordinal, tag, project.owner, project.id]);
  const validate = useMemo(() => (value: unknown) => validateProjectIteration(value, choice.date), [choice.date]);
  const { data, error } = useProduct<SpecialProjectIteration>(runtime.client, request, active, validate, track?.source);
  if (!recorded) return <p className="project-window-withheld" title="This iteration's hour is not recorded in this branch.">Not recorded</p>;
  if (error) return <p className="project-window-withheld" title={error.message}>Not recorded</p>;
  if (!data) return null;
  const refusals = Object.keys(data.refusals);
  if (data.unavailable) return <p className="project-window-withheld" title={data.unavailable}>Not recorded</p>;
  if (data.noChanges) return <div className="project-window-output"><p className="game-text">No changes</p></div>;
  return <>
    {data.sections.length > 0 && <GameIcons runs={data.sections.flatMap(section => [...section.heading ?? [], ...section.runs])}>
      <div className="project-window-output">{data.sections.map((section, at) => <div key={at}>
        {section.heading && <span className="project-window-output-label"><GameRuns runs={trimLabel(section.heading)} /></span>}
        <GameText runs={trimValue(section.runs)} onOpen={onOpen} />
      </div>)}</div></GameIcons>}
    {refusals.length > 0 && <p className="project-window-withheld" title={refusals.join('\n')}>Part of this reward can't be shown</p>}
  </>;
}
