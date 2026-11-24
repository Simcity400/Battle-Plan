import { countryFromKey } from '../../dashboard/countryKey.ts';
import { createContext, useContext, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { GatedHour } from '../../dashboard/regionTime';
import { useProduct } from '../../dashboard/products';
import { productRequest } from '../../dashboard/request.ts';
import { detailTexts, validateProjectDetails } from './projectDetailsData';
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

export interface Asked { runtime: WidgetRuntime; widget: WidgetSpec; tag: string; active: boolean; readHour: GatedHour }

export function usePinnedDetails({ runtime, widget, tag, active, readHour }: Asked) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = useMemo(() => (track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'special-project-details',
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour: readHour } }) : null), [track, tag, readHour]);
  return useProduct<SpecialProjectDetails>(runtime.client, request, active, validateProjectDetails, track?.source);
}

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

export default function ProjectDetails({ onWarnings, ...asked }: Asked & { onWarnings: (warnings: readonly string[]) => void }) {
  const { data, error, retry } = usePinnedDetails(asked);
  const values = useMemo(() => data ? detailTexts(data) : new Map<string, string>(), [data]);
  const warnings = JSON.stringify(data?.warnings ?? []);
  useEffect(() => onWarnings(JSON.parse(warnings) as string[]), [warnings, onWarnings]);
  useEffect(() => () => onWarnings([]), [onWarnings]);
  return <div className="project-details">
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : !data ? <WidgetState kind="loading" label="Loading project details…" /> : <DetailValues.Provider value={values}><Shown details={data} /></DetailValues.Provider>}
  </div>;
}
