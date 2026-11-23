import { countryFromKey } from '../../dashboard/countryKey.ts';
import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { RefObject } from 'react';
import WidgetState from '../../components/WidgetState';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import { panelFeed } from '../../dashboard/panelFeed';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import type { WidgetSpec } from '../../dashboard/model';
import { dayLabel } from '../../charts/draw';
import { detailsStructure, detailTexts, effectsStructure, validateProjectDetails, validateProjectEffects } from './projectDetailsData';
import type { ProgramDetails, ProjectEffects, ProjectLabel, ProjectRuntimeDetails, SpecialProjectDetails } from '../../generated/dashboard';

const Name = ({ label }: { label: ProjectLabel }) => <span data-tooltip={contentTitleHint(label.token)}>{contentTitle(label.name, label.token, 'Name unavailable')}</span>;
const DetailValues = createContext<ReadonlyMap<string, string>>(new Map());
const Painted = ({ at }: { at: string }) => <span data-paint={at}>{useContext(DetailValues).get(at)}</span>;

function Effects({ prefix, value, state }: { prefix: string; value: ProjectEffects | null | undefined; state: string | null }) {
  if (state) return <p>{state}</p>;
  if (!value) return <p>Effects unavailable.</p>;
  return <div data-effects-ink="">{value.text && <pre className="project-effect-text">{value.text.split('\n').map((line, i) =>
    <span key={i}>{i > 0 && '\n'}<span data-prose="" data-paint={prefix + ':' + i}>{line}</span></span>)}</pre>}
    {Object.entries(value.refusals).map(([reason, count]) => <p className="project-detail-missing" key={reason}>
      <span data-paint={prefix + ':reason:' + reason} data-prose="">{reason} ({count})</span></p>)}</div>;
}
const Count = ({ roster, label }: { roster: readonly unknown[] | null; label: string }) => roster === null ? null : <span className="pill" data-panel-label={label}>{roster.length}</span>;

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

function Project(props: Asked & { project: ProjectRuntimeDetails }) {
  const { project } = props;
  const box = useRef<HTMLDetailsElement>(null);
  const [opened, setOpened] = useState(false);
  const [open, setOpen] = useState(false);
  return <details className="project-detail" ref={box} onToggle={event => {
    const shown = event.currentTarget.open;
    if (shown) setOpened(true);
    setOpen(shown);
  }}>
    <summary><Name label={project.label} /><Painted at={`j:${project.owner}:${project.id}:read`} /></summary>
    {opened && <ProjectBody {...props} box={box} open={open} />}
  </details>;
}

function ProjectBody({ project, runtime, widget, tag, active, open, box }: Asked & {
  project: ProjectRuntimeDetails; open: boolean; box: RefObject<HTMLDetailsElement | null>;
}) {
  const effects = useHeadProduct(runtime, useGate(runtime, widget), 'special-project-effects', open ? { country: countryFromKey(tag), owner: project.owner, project: project.id } : null,
    { active: active && open, validate: validateProjectEffects, structure: effectsStructure });
  const state = effects.error ? effects.error.message : !effects.data ? 'Loading effects…' : effects.data.unavailable;
  useLayoutEffect(() => { effects.root(box.current); }, [effects.root, box]);
  useDrawnPanel(box, runtime.clock, panelFeed('special-project-effects', effects, effectsStructure, widget, runtime), active && open);
  return <>
    {!project.pendingKnown ? <p>Pending reward unavailable.</p> : !project.pending ? <p>No pending reward choice.</p> : <div>
      <h4><Name label={project.pending.label} /></h4>{project.pending.description && <p>{project.pending.description}</p>}
      {project.pending.unavailable && <p>{project.pending.unavailable}</p>}
      {project.pending.options.map((option, index) => <details key={index}><summary><Name label={option.label} /></summary>
        <Effects prefix={String(index)} value={effects.data?.options[index]} state={state} /></details>)}
    </div>}
    <details><summary>Chosen reward options</summary>{project.choices === null ? <p>Choice history unavailable.</p> : project.choices.length === 0 ? <p>No chosen reward options recorded.</p> : <ul>{project.choices.map(choice => <li key={choice.ordinal}>
      {dayLabel(choice.date)} · <Name label={choice.reward} /> · <Name label={choice.option} />
      <p>Character reference {choice.characterReference} · Province {choice.province} · Progress {choice.progress}{choice.scientist !== null && ` · Scientist ${choice.scientist}`}{choice.country !== null && ` · Country ${choice.country}`}{choice.state !== null && ` · State ${choice.state}`}</p>
    </li>)}</ul>}</details>
    <details><summary>Project output</summary><Effects prefix="output" value={effects.data?.output} state={state} /></details>
    <details><summary>Variables</summary>{project.variables === null ? <p>Variables unavailable.</p> : project.variables.length === 0 ? <p>No project variables.</p> : <dl>{project.variables.map((variable, index) => <div key={index}><dt>{variable.name}</dt><dd><Painted at={`j:${project.owner}:${project.id}:var:${index}`} /></dd></div>)}</dl>}
      {project.seed !== null && <p>Variable random seed: <Painted at={`j:${project.owner}:${project.id}:seed`} /></p>}</details>
  </>;
}

function Shown({ details, asked }: { details: SpecialProjectDetails; asked: Asked }) {
  return <>
    <details open><summary>Scientists <Count label="Scientists" roster={details.scientists} /></summary>
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
    <details open><summary>Facility assignments <Count label="Facility assignments" roster={details.programs} /></summary>
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
    <details open><summary>Project rewards and variables <Count label="Project rewards and variables" roster={details.projects} /></summary>
      {details.projects === null && <p>Project details unavailable at this hour.</p>}
      {details.projects?.map(project => <Project key={`${project.owner}:${project.id}`} project={project} {...asked} />)}
    </details>
    {details.unavailable.length > 0 && <details><summary>Unavailable details</summary><ul>{details.unavailable.map((reason, index) => <li key={index}>{reason}</li>)}</ul></details>}
  </>;
}

export default function ProjectDetails({ runtime, widget, tag, active, onWarnings }: Asked & { onWarnings: (warnings: readonly string[]) => void }) {
  const box = useRef<HTMLDivElement | null>(null);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'special-project-details', { country: countryFromKey(tag) },
    { active, validate: validateProjectDetails, structure: detailsStructure });
  const { data, error, retry, root } = head;
  useDrawnPanel(box, runtime.clock, panelFeed('special-project-details', head, detailsStructure, widget, runtime), active);
  const values = useMemo(() => data ? detailTexts(data) : new Map<string, string>(), [data]);
  const warnings = JSON.stringify(data?.warnings ?? []);
  useEffect(() => onWarnings(JSON.parse(warnings) as string[]), [warnings, onWarnings]);
  useEffect(() => () => onWarnings([]), [onWarnings]);
  const ref = useCallback((element: HTMLDivElement | null) => { box.current = element; root(element); }, [root]);
  return <div className="project-details" ref={ref} data-structure={data ? detailsStructure(data) : undefined}>
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : !data ? <WidgetState kind="loading" label="Loading project details…" /> : <DetailValues.Provider value={values}><Shown details={data} asked={{ runtime, widget, tag, active }} /></DetailValues.Provider>}
  </div>;
}
