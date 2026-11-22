import { countryFromKey } from '../../dashboard/countryKey.ts';
import { createContext, useCallback, useContext, useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import { panelFeed } from '../../dashboard/panelFeed';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { WidgetSpec } from '../../dashboard/model';
import { detailsStructure, detailTexts, validateProjectDetails } from './projectDetailsData';
import type { ProgramDetails, ProjectLabel, SpecialProjectDetails } from '../../generated/dashboard';

export const Name = ({ label }: { label: ProjectLabel }) => <span data-tooltip={contentTitleHint(label.token)}>{contentTitle(label.name, label.token, 'Name unavailable')}</span>;
export const DetailValues = createContext<ReadonlyMap<string, string>>(new Map());
export const Painted = ({ at }: { at: string }) => <span data-paint={at}>{useContext(DetailValues).get(at)}</span>;

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

export interface Asked { runtime: WidgetRuntime; widget: WidgetSpec; tag: string; active: boolean }

function Shown({ details }: { details: SpecialProjectDetails }) {
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
    {details.unavailable.length > 0 && <details><summary>Unavailable details</summary><ul>{details.unavailable.map((reason, index) => <li key={index}>{reason}</li>)}</ul></details>}
  </>;
}

export default function ProjectDetails({ runtime, widget, tag, active, onWarnings }: Asked & { onWarnings: (warnings: readonly string[]) => void }) {
  const box = useRef<HTMLDivElement | null>(null);
  const head = useHeadProduct<SpecialProjectDetails>(runtime, widget, 'special-project-details', { country: countryFromKey(tag) },
    { active, validate: validateProjectDetails, structure: detailsStructure });
  const { data, error, retry, root } = head;
  useDrawnPanel(box, runtime.clock, panelFeed('special-project-details', head, detailsStructure, widget, runtime), active);
  const values = useMemo(() => data ? detailTexts(data) : new Map<string, string>(), [data]);
  const warnings = JSON.stringify(data?.warnings ?? []);
  useEffect(() => onWarnings(JSON.parse(warnings) as string[]), [warnings, onWarnings]);
  useEffect(() => () => onWarnings([]), [onWarnings]);
  const ref = useCallback((element: HTMLDivElement | null) => { box.current = element; root(element); }, [root]);
  return <div className="project-details" ref={ref} data-structure={data ? detailsStructure(data) : undefined}>
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : !data ? <WidgetState kind="loading" label="Loading project details…" /> : <DetailValues.Provider value={values}><Shown details={data} /></DetailValues.Provider>}
  </div>;
}
