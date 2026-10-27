import type { SpecialProjectDetails, ProjectEffects, ProjectLabel, ProgramDetails } from '../../generated/dashboard';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import { dayLabel } from '../../charts/draw';
import { researchNumber } from '../research/researchBreakdown';

const Name = ({ label }: { label: ProjectLabel }) => <span data-tooltip={contentTitleHint(label.token)}>{contentTitle(label.name, label.token, 'Name unavailable')}</span>;
const number = (raw: string) => researchNumber(BigInt(raw));
function Effects({ value }: { value: ProjectEffects | null }) {
  if (!value) return <p>Effects unavailable.</p>;
  return <>{value.text && <pre className="project-effect-text">{value.text}</pre>}
    {Object.entries(value.refusals).map(([reason, count]) => <p className="project-detail-missing" key={reason}>{reason} ({count})</p>)}</>;
}
function Facility({ program }: { program: ProgramDetails }) {
  const facility = program.facility;
  if (!facility) return <p>Facility speed and specialization contributions unavailable.</p>;
  return <div><p>Research speed: {facility.speedRaw === null ? 'Unavailable' : `${number(facility.speedRaw)}×`}</p>
    {facility.unavailable && <p>{facility.unavailable}</p>}
    <ul>{facility.sources.map((source, index) => <li key={index}>
      {source.kind}{source.label && <> · <Name label={source.label} /></>}{source.modifier && <> · <Name label={source.modifier} /></>}
      {source.scientistReference !== '0' && <> · {source.assignment === 0 ? 'Lead' : 'Supporting'}: {program.assignments?.[source.assignment]?.person.name ?? 'Scientist unavailable'}</>}
      : {number(source.valueRaw)}{source.unit === 'factor' ? '×' : source.unit === 'bonus' ? ' research bonus' : ''}
    </li>)}</ul>
  </div>;
}
export default function ProjectDetails({ details }: { details?: SpecialProjectDetails }) {
  if (!details) return null;
  return <div className="project-details">
    <details open><summary>Scientists <span className="pill">{details.scientists.length}</span></summary>
      {details.scientists.length === 0 && <p>No scientists recorded at this hour.</p>}
      {details.scientists.map(scientist => <details className="project-detail" key={scientist.character}>
        <summary>{scientist.person.name ?? 'Scientist name unavailable'} · {scientist.active === null ? 'State unavailable' : scientist.active === 0 ? 'Unassigned' : 'Assigned'}</summary>
        {scientist.person.portrait && <img className="project-detail-portrait" src={scientist.person.portrait} alt="" />}
        <p>{scientist.injuryDays === null ? 'Injury recovery unavailable.' : scientist.injuryDays > 0 ? `${scientist.injuryDays} injury recovery days` : 'No injury recovery pending'}
          {scientist.injuryHour !== null && ` · read ${dayLabel(scientist.injuryHour)}`}</p>
        {scientist.skills === null ? <p>Skills unavailable.</p> : scientist.skills.length === 0 ? <p>No specializations.</p> : <ul>{scientist.skills.map((skill, index) => <li key={index}>
          <Name label={skill.specialization} /> · Level {skill.level} · {number(skill.experienceRaw)} experience</li>)}</ul>}
        <p>{scientist.visible === null ? 'Roster visibility unavailable' : scientist.visible ? 'Visible in scientist roster' : 'Hidden from scientist roster'}{scientist.visibilityHour !== null && ` · read ${dayLabel(scientist.visibilityHour)}`}</p>
        {Object.entries(scientist.stateRefusals).map(([reason, count]) => <p key={reason}>{reason} ({count})</p>)}
        <p>Traits: {scientist.traits === null ? 'Unavailable' : scientist.traits.length === 0 ? 'None' : scientist.traits.map((trait, index) => <span key={index}>{index > 0 && ', '}<Name label={trait} /></span>)}</p>
      </details>)}
    </details>
    <details><summary>Specialization progress</summary>{details.breakthroughs.length === 0 ? <p>No specialization progress recorded.</p> : <ul>{details.breakthroughs.map((progress, index) => <li key={index}><Name label={progress.specialization} />: {progress.points / 10000} breakthrough progress · read {dayLabel(progress.readHour)}</li>)}</ul>}</details>
    <details open><summary>Facility assignments <span className="pill">{details.programs.length}</span></summary>
      {details.programs.length === 0 && <p>No facility assignments recorded.</p>}
      {details.programs.map(program => <details className="project-detail" key={program.id}>
        <summary>Facility in province {program.province} · read {dayLabel(program.readHour)}</summary>
        <Facility program={program} />
        {program.assignments === null ? <p>Assignments unavailable.</p> : <ul>{program.assignments.map(assignment => <li key={assignment.ordinal}>
          <b>{assignment.ordinal === 0 ? 'Lead' : 'Supporting'}</b>: {assignment.person.name ?? (assignment.person.assigned === false ? 'Unassigned' : 'Scientist unavailable')}
          {assignment.timed !== 0 && <span> · Delayed unassignment: {assignment.target - assignment.current} days remaining ({assignment.current} / {assignment.target})</span>}
        </li>)}</ul>}
      </details>)}
    </details>
    <details open><summary>Project rewards and variables <span className="pill">{details.projects.length}</span></summary>
      {details.projects.map(project => <details className="project-detail" key={`${project.owner}:${project.id}`}>
        <summary><Name label={project.label} /> · read {dayLabel(project.readHour)}</summary>
        {!project.pendingKnown ? <p>Pending reward unavailable.</p> : !project.pending ? <p>No pending reward choice.</p> : <div>
          <h4><Name label={project.pending.label} /></h4>{project.pending.description && <p>{project.pending.description}</p>}
          {project.pending.unavailable && <p>{project.pending.unavailable}</p>}
          {project.pending.options.map((option, index) => <details key={index}><summary><Name label={option.label} /></summary><Effects value={option.effects} /></details>)}
        </div>}
        <details><summary>Chosen reward options</summary>{project.choices === null ? <p>Choice history unavailable.</p> : project.choices.length === 0 ? <p>No chosen reward options recorded.</p> : <ul>{project.choices.map(choice => <li key={choice.ordinal}>
          {dayLabel(choice.date)} · <Name label={choice.reward} /> · <Name label={choice.option} />
          <p>Character reference {choice.characterReference} · Province {choice.province} · Progress {choice.progress}{choice.scientist !== null && ` · Scientist ${choice.scientist}`}{choice.country !== null && ` · Country ${choice.country}`}{choice.state !== null && ` · State ${choice.state}`}</p>
        </li>)}</ul>}</details>
        <details><summary>Project output</summary><Effects value={project.output} /></details>
        <details><summary>Variables</summary>{project.variables === null ? <p>Variables unavailable.</p> : project.variables.length === 0 ? <p>No project variables.</p> : <dl>{project.variables.map((variable, index) => <div key={index}><dt>{variable.name}</dt><dd>{number(variable.valueRaw)}</dd></div>)}</dl>}
          {project.seed !== null && <p>Variable random seed: {project.seed}</p>}</details>
      </details>)}
    </details>
    {details.unavailable.length > 0 && <details><summary>Unavailable details</summary><ul>{details.unavailable.map((reason, index) => <li key={index}>{reason}</li>)}</ul></details>}
  </div>;
}
