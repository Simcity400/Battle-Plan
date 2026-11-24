import { productRequest } from '../../dashboard/request.ts';
import { useGate, type Gate } from '../../dashboard/regionGate';
import { useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import '../../components/Dossier.css';
import { dayLabel } from '../../charts/draw';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { formatDateTime } from '../../dashboard/calendar';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { OpenedLink } from '../research/TechnologyWindow';
import GameText, { GameArtFor, GameBlocks, GameIcons, GameRuns, type EntityLink } from '../research/GameText';
import { trimLabel, trimValue } from '../research/gameRuns';
import { DetailValues, Name, Painted } from './ProjectDetails';
import { usePinnedDetails, type Asked } from './ProjectDetails';
import { detailTexts, effectsRead, optionEffects, validateProjectEffects, validateProjectIteration } from './projectDetailsData';
import { episodeBound, iterationCount, lastScientist, readLabel, runtimeProject, scientistName, statusRead } from './specialProjectsData';
import type { ProjectCard, RuntimeMatch } from './specialProjectsData';
import type { ProjectChoice, ProjectEffects, ProjectRuntimeDetails, SpecialProjectEffects, SpecialProjectIteration, ProjectStatus, SpecialProjectsReply } from '../../generated/dashboard';

const STATES = { running: 'In progress', paused: 'Paused', unknown: 'State unknown', completed: 'Completed', cancelled: 'Cancelled' } as const;
const STATE_TONES = { running: '', paused: ' is-paused', unknown: ' is-quiet', completed: ' is-done', cancelled: ' is-quiet' } as const;

type WindowProps = Asked & { reply: SpecialProjectsReply; card: ProjectCard | null; hour: number; missing: string };

type DossierProps = WindowProps;

export function ProjectDossier({ reply, card, hour, missing, ...asked }: DossierProps) {
  return <GameArtFor runtime={asked.runtime} widget={asked.widget} active={asked.active}>{card ? <div className="dossier is-blueprint">
    <Summary reply={reply} card={card} hour={hour} />
    <Runtime {...asked} reply={reply} card={card} hour={hour} />
  </div> : <div className="dossier-body"><WidgetState kind="unavailable" message={missing} /></div>}</GameArtFor>;
}


function Summary({ reply, card, hour }: { reply: SpecialProjectsReply; card: ProjectCard; hour: number }) {
  const { project, run, info } = card;
  const done = run.state === 'completed';
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
    <div className={`dossier-progress${done ? ' is-done' : ''}`} data-project={done ? undefined : project.id}
      style={{ '--project-progress': Math.max(0, run.progress) / 100 } as CSSProperties}>
      {done ? <strong className="dossier-progress-value">{read ? `Completed · ${read}` : 'Completed'}</strong> : <>
        <strong className="dossier-progress-value" data-paint="progress">{run.progress}%</strong>
        <span className="special-project-progress" aria-hidden="true"><span /></span>
      </>}
    </div>
  </>;
}

function Runtime({ reply, card, hour, ...asked }: Asked & { reply: SpecialProjectsReply; card: ProjectCard; hour: number }) {
  const { data, error, retry } = usePinnedDetails(asked);
  const values = useMemo(() => data ? detailTexts(data) : new Map<string, string>(), [data]);
  const match = data ? runtimeProject(data, card, hour, episodeBound(reply, card.project, null)) : null;
  return <div className="dossier-body">
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : !data || !match ? <WidgetState kind="loading" label="Loading project rewards…" />
      : typeof match === 'string' ? <p className="dossier-missing">{match}</p>
        : <DetailValues.Provider value={values}><Rewards {...asked} hour={data.hour} match={match} run={card.run} /></DetailValues.Provider>}
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

function Rewards({ hour, match, run, ...asked }: Asked & { hour: number; match: RuntimeMatch; run: ProjectStatus }) {
  const { runtime, widget, tag, active } = asked;
  const { project } = match;
  const done = run.state === 'completed';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const effectsRequest = useMemo(() => (track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'special-project-effects',
    params: { branchId: track.branch.id, country: countryFromKey(tag), owner: project.owner, project: project.id, hour: asked.readHour } }) : null),
  [track, tag, project.owner, project.id, asked.readHour]);
  const effects = useProduct<SpecialProjectEffects>(runtime.client, effectsRequest, active, validateProjectEffects, track?.source);
  const read = effects.error ? effects.error.message : !effects.data ? 'Loading effects…' : effectsRead(hour, project, effects.data);
  const state = read ?? (effects.data?.output ? null : effects.data?.unavailable ?? null);
  const options = read ?? (effects.data ? optionEffects(project, effects.data) : 'Loading effects…');
  const [opened, setOpened] = useState<EntityLink | null>(null);
  const at = `j:${project.owner}:${project.id}`;
  return <div className="project-window-sections">
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
    <Iterations {...asked} project={project} choices={match.choices} run={run} onOpen={setOpened} />
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
  const gate = useGate(runtime, widget);
  const listed = (shown: readonly ProjectChoice[]) => <ol className="project-window-timeline">{shown.map(choice => <li key={choice.ordinal}>
    <time>{dayLabel(choice.date)}</time>
    <p className="project-window-choice-name"><Name label={choice.reward} /><span className="project-window-option"> · <Name label={choice.option} /></span></p>
    <IterationOutput runtime={runtime} gate={gate} track={track} tag={tag} active={active} project={project} choice={choice} onOpen={onOpen} />
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

function IterationOutput({ runtime, gate, track, tag, active, project, choice, onOpen }: Pick<Asked, 'runtime' | 'tag' | 'active'> & {
  gate: Pick<Gate, 'recorded'>; track: BoundTrack | undefined; project: ProjectRuntimeDetails; choice: ProjectChoice; onOpen: (link: EntityLink) => void;
}) {
  const at = gate.recorded(choice.date), read = gate.recorded(project.historyReadHour);
  const recorded = !!track && at !== null;
  const request = useMemo(() => track && at !== null && read !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'special-project-iteration',
    params: { branchId: track.branch.id, hour: at, country: countryFromKey(tag), owner: project.owner, project: project.id, ordinal: choice.ordinal, read } }) : null,
  [track, at, read, choice.ordinal, tag, project.owner, project.id]);
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
