import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { WidgetSpec } from '../../dashboard/model';
import { dayLabel } from '../../charts/draw';
import { detailsStructure, paintDetails, validateProjectDetails, validateProjectEffects } from './projectDetailsData';
import type { ProgramDetails, ProjectEffects, ProjectLabel, ProjectRuntimeDetails, SpecialProjectDetails, SpecialProjectEffects } from '../../generated/dashboard';

const Name = ({ label }: { label: ProjectLabel }) => <span data-tooltip={contentTitleHint(label.token)}>{contentTitle(label.name, label.token, 'Name unavailable')}</span>;
/** A value the game refreshes daily: its text is written by the painter, never by a render. */
const Painted = ({ at }: { at: string }) => <span data-paint={at} />;
const effectsStructure = (reply: SpecialProjectEffects) => JSON.stringify(reply);

function Effects({ value, state }: { value: ProjectEffects | null | undefined; state: string | null }) {
  if (state) return <p>{state}</p>;
  if (!value) return <p>Effects unavailable.</p>;
  return <>{value.text && <pre className="project-effect-text">{value.text}</pre>}
    {Object.entries(value.refusals).map(([reason, count]) => <p className="project-detail-missing" key={reason}>{reason} ({count})</p>)}</>;
}
/** An unavailable roster shows no count: it is unknown, not empty. */
const Count = ({ roster }: { roster: readonly unknown[] | null }) => roster === null ? null : <span className="pill">{roster.length}</span>;

function Facility({ program }: { program: ProgramDetails }) {
  const facility = program.facility;
  if (!facility) return <p>Facility speed and specialization contributions unavailable.</p>;
  return <div><p><Painted at={`p:${program.id}:speed`} /></p>
    {facility.unavailable && <p>{facility.unavailable}</p>}
    <ul>{facility.sources.map((source, index) => <li key={index}>
      {source.kind}{source.label && <> · <Name label={source.label} /></>}{source.modifier && <> · <Name label={source.modifier} /></>}
      {source.scientistReference !== '0' && <> · {source.assignment === 0 ? 'Lead' : 'Supporting'}: {program.assignments?.[source.assignment]?.person.name ?? 'Scientist unavailable'}</>}
      <Painted at={`p:${program.id}:source:${index}`} />
    </li>)}</ul>
  </div>;
}

interface Asked { runtime: WidgetRuntime; widget: WidgetSpec; tag: string; active: boolean }

/** One project's rewards, choices, output and variables; its effects text is asked only while it is open, as it reads
 * every table scripted effects can name. */
function Project({ project, runtime, widget, tag, active }: Asked & { project: ProjectRuntimeDetails }) {
  const [open, setOpen] = useState(false);
  const effects = useHeadProduct<SpecialProjectEffects>(runtime, widget, 'special-project-effects', open ? { tag, owner: project.owner, project: project.id } : null,
    { active: active && open, validate: validateProjectEffects, structure: effectsStructure });
  const state = effects.error ? effects.error.message : !effects.data ? 'Loading effects…' : effects.data.unavailable;
  return <details className="project-detail" ref={effects.root} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary><Name label={project.label} /><Painted at={`j:${project.owner}:${project.id}:read`} /></summary>
    {!project.pendingKnown ? <p>Pending reward unavailable.</p> : !project.pending ? <p>No pending reward choice.</p> : <div>
      <h4><Name label={project.pending.label} /></h4>{project.pending.description && <p>{project.pending.description}</p>}
      {project.pending.unavailable && <p>{project.pending.unavailable}</p>}
      {project.pending.options.map((option, index) => <details key={index}><summary><Name label={option.label} /></summary>
        <Effects value={effects.data?.options[index]} state={state} /></details>)}
    </div>}
    <details><summary>Chosen reward options</summary>{project.choices === null ? <p>Choice history unavailable.</p> : project.choices.length === 0 ? <p>No chosen reward options recorded.</p> : <ul>{project.choices.map(choice => <li key={choice.ordinal}>
      {dayLabel(choice.date)} · <Name label={choice.reward} /> · <Name label={choice.option} />
      <p>Character reference {choice.characterReference} · Province {choice.province} · Progress {choice.progress}{choice.scientist !== null && ` · Scientist ${choice.scientist}`}{choice.country !== null && ` · Country ${choice.country}`}{choice.state !== null && ` · State ${choice.state}`}</p>
    </li>)}</ul>}</details>
    <details><summary>Project output</summary><Effects value={effects.data?.output} state={state} /></details>
    <details><summary>Variables</summary>{project.variables === null ? <p>Variables unavailable.</p> : project.variables.length === 0 ? <p>No project variables.</p> : <dl>{project.variables.map((variable, index) => <div key={index}><dt>{variable.name}</dt><dd><Painted at={`j:${project.owner}:${project.id}:var:${index}`} /></dd></div>)}</dl>}
      {project.seed !== null && <p>Variable random seed: {project.seed}</p>}</details>
  </details>;
}

function Shown({ details, asked }: { details: SpecialProjectDetails; asked: Asked }) {
  return <>
    <details open><summary>Scientists <Count roster={details.scientists} /></summary>
      {details.scientists === null ? <p>Scientist roster unavailable at this hour.</p> : details.scientists.length === 0 && <p>No scientists recorded at this hour.</p>}
      {details.scientists?.map(scientist => <details className="project-detail" key={scientist.character}>
        <summary>{scientist.person.name ?? 'Scientist name unavailable'} · <Painted at={`s:${scientist.character}:state`} /></summary>
        {scientist.person.portrait && <img className="project-detail-portrait" src={scientist.person.portrait} alt="" />}
        <p><Painted at={`s:${scientist.character}:injury`} /></p>
        {scientist.skills === null ? <p>Skills unavailable.</p> : scientist.skills.length === 0 ? <p>No specializations.</p> : <ul>{scientist.skills.map((skill, index) => <li key={index}>
          <Name label={skill.specialization} /><Painted at={`s:${scientist.character}:skill:${index}`} /></li>)}</ul>}
        <p><Painted at={`s:${scientist.character}:visibility`} /></p>
        {Object.entries(scientist.stateRefusals).map(([reason, count]) => <p key={reason}>{reason} ({count})</p>)}
        <p>Traits: {scientist.traits === null ? 'Unavailable' : scientist.traits.length === 0 ? 'None' : scientist.traits.map((trait, index) => <span key={index}>{index > 0 && ', '}<Name label={trait} /></span>)}</p>
      </details>)}
    </details>
    <details><summary>Specialization progress</summary>{details.breakthroughs === null ? <p>Specialization progress unavailable.</p> : details.breakthroughs.length === 0 ? <p>No specialization progress recorded.</p>
      : <ul>{details.breakthroughs.map((progress, index) => <li key={index}><Name label={progress.specialization} /><Painted at={`b:${index}`} /></li>)}</ul>}</details>
    <details open><summary>Facility assignments <Count roster={details.programs} /></summary>
      {details.programs === null ? <p>Facility assignments unavailable at this hour.</p> : details.programs.length === 0 && <p>No facility assignments recorded.</p>}
      {details.programs?.map(program => <details className="project-detail" key={program.id}>
        <summary><Painted at={`p:${program.id}:read`} /></summary>
        <Facility program={program} />
        {program.assignments === null ? <p>Assignments unavailable.</p> : <ul>{program.assignments.map(assignment => <li key={assignment.ordinal}>
          <b>{assignment.ordinal === 0 ? 'Lead' : 'Supporting'}</b>: {assignment.person.name ?? (assignment.person.assigned === false ? 'Unassigned' : 'Scientist unavailable')}
          <Painted at={`p:${program.id}:assignment:${assignment.ordinal}`} />
        </li>)}</ul>}
      </details>)}
    </details>
    <details open><summary>Project rewards and variables <Count roster={details.projects} /></summary>
      {details.projects === null && <p>Project details unavailable at this hour.</p>}
      {details.projects?.map(project => <Project key={`${project.owner}:${project.id}`} project={project} {...asked} />)}
    </details>
    {details.unavailable.length > 0 && <details><summary>Unavailable details</summary><ul>{details.unavailable.map((reason, index) => <li key={index}>{reason}</li>)}</ul></details>}
  </>;
}

/** The scientists, facilities and project rewards of `tag` at the widget's hour: a pushed panel while following, which
 * renders only when its structure changes; the values the game refreshes daily are painted ([`detailTexts`]). */
export default function ProjectDetails({ runtime, widget, tag, active, onWarnings }: Asked & { onWarnings: (warnings: readonly string[]) => void }) {
  const box = useRef<HTMLDivElement | null>(null);
  const paint = useCallback((reply: SpecialProjectDetails) => paintDetails(box.current, reply), []);
  const { data, error, retry, root } = useHeadProduct<SpecialProjectDetails>(runtime, widget, 'special-project-details', { tag },
    { active, validate: validateProjectDetails, structure: detailsStructure, paint });
  const warnings = JSON.stringify(data?.warnings ?? []);
  useEffect(() => onWarnings(JSON.parse(warnings) as string[]), [warnings, onWarnings]);
  useEffect(() => () => onWarnings([]), [onWarnings]);
  useLayoutEffect(() => paintDetails(box.current, data), [data]);
  const ref = useCallback((element: HTMLDivElement | null) => { box.current = element; root(element); }, [root]);
  return <div className="project-details" ref={ref}>
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : !data ? <WidgetState kind="loading" label="Loading project details…" /> : <Shown details={data} asked={{ runtime, widget, tag, active }} />}
  </div>;
}
