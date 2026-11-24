import { dayLabel } from '../../charts/draw.ts';
import { researchNumber } from '../research/researchBreakdown.ts';
import { validGameRuns } from '../research/gameRuns.ts';
import type { ProjectEffects, ProjectRuntimeDetails, ScientistDetails, SpecialProjectDetails, SpecialProjectEffects, SpecialProjectIteration } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const integer = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
const uint = (value: unknown) => integer(value) && value >= 0 && value <= 0xffffffff;
const signed = (value: unknown) => integer(value) && value >= -0x80000000 && value <= 0x7fffffff;
const nullable = (value: unknown) => value === null || typeof value === 'string';
const strings = (value: unknown) => Array.isArray(value) && value.every(item => typeof item === 'string');
const raw = (value: unknown) => typeof value === 'string' && /^-?(0|[1-9][0-9]{0,18})$/.test(value) && BigInt(value) >= -(1n << 63n) && BigInt(value) < (1n << 63n);
const reference = (value: unknown) => typeof value === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(value) && BigInt(value) < 1n << 64n;
const label = (value: unknown) => object(value) && typeof value.token === 'string' && nullable(value.name);
const person = (value: unknown) => object(value) && (value.assigned === null || typeof value.assigned === 'boolean') && (value.id === null || uint(value.id)) && nullable(value.name) && nullable(value.portrait) && nullable(value.missing);
const counts = (value: unknown) => object(value) && Object.values(value).every(count => integer(count) && count > 0);
const effects = (value: unknown) => object(value) && validGameRuns(value.runs) && counts(value.refusals);
const byte = (value: unknown) => integer(value) && value >= 0 && value <= 255;
const facility = (value: unknown) => object(value) && (value.speedRaw === null || raw(value.speedRaw)) && nullable(value.unavailable)
  && Array.isArray(value.sources) && value.sources.every(source => object(source) && typeof source.kind === 'string' && typeof source.unit === 'string'
    && (source.label === null || label(source.label)) && (source.modifier === null || label(source.modifier)) && uint(source.assignment)
    && reference(source.scientistReference) && raw(source.valueRaw));
const reward = (value: unknown) => object(value) && label(value.label) && (value.ordinal === null || uint(value.ordinal)) && nullable(value.description) && nullable(value.unavailable) && Array.isArray(value.options)
  && value.options.every(option => object(option) && label(option.label));
const roster = (value: unknown): value is unknown[] | null => value === null || Array.isArray(value);
const readBy = (value: unknown, hour: number) => integer(value) && value <= hour;

export function validateProjectDetails(value: unknown): SpecialProjectDetails {
  const bad = () => { throw new Error('Invalid special-project details'); };
  if (!object(value) || !integer(value.hour) || !roster(value.scientists) || !roster(value.programs) || !roster(value.projects) || !roster(value.breakthroughs)
    || !strings(value.unavailable) || !strings(value.warnings)) return bad();
  const hour = value.hour;
  for (const scientist of value.scientists ?? []) if (!object(scientist) || !uint(scientist.character) || !person(scientist.person)
    || !(scientist.active === null || byte(scientist.active)) || !(scientist.injuryDays === null || signed(scientist.injuryDays))
    || !(scientist.visible === null || typeof scientist.visible === 'boolean') || !(scientist.visibilityHour === null || readBy(scientist.visibilityHour, hour))
    || !counts(scientist.stateRefusals) || !(scientist.injuryHour === null || readBy(scientist.injuryHour, hour))
    || !(scientist.skills === null || Array.isArray(scientist.skills) && scientist.skills.every(skill => object(skill) && label(skill.specialization) && signed(skill.level) && raw(skill.experienceRaw)))
    || !(scientist.traits === null || Array.isArray(scientist.traits) && scientist.traits.every(label))) return bad();
  for (const progress of value.breakthroughs ?? []) if (!object(progress) || !label(progress.specialization) || !signed(progress.points) || !readBy(progress.readHour, hour)) return bad();
  for (const program of value.programs ?? []) if (!object(program) || !uint(program.id) || !uint(program.province) || !readBy(program.readHour, hour) || !(program.facility === null || facility(program.facility))
    || !(program.assignments === null || Array.isArray(program.assignments) && program.assignments.every((assignment, ordinal) => object(assignment) && assignment.ordinal === ordinal
      && reference(assignment.reference) && person(assignment.person) && signed(assignment.target) && signed(assignment.current) && byte(assignment.timed)))) return bad();
  for (const project of value.projects ?? []) if (!object(project) || !uint(project.owner) || !uint(project.id) || !label(project.label) || !readBy(project.readHour, hour)
    || !(project.historyReadHour === null ? project.choices === null : readBy(project.historyReadHour, hour) && project.choices !== null)
    || typeof project.pendingKnown !== 'boolean' || !(project.pending === null || project.pendingKnown && reward(project.pending)) || !(project.seed === null || uint(project.seed))
    || !(project.variables === null || Array.isArray(project.variables) && project.variables.every(variable => object(variable) && typeof variable.name === 'string' && raw(variable.valueRaw)))
    || !(project.choices === null || Array.isArray(project.choices) && project.choices.every(choice => object(choice) && uint(choice.ordinal) && integer(choice.date) && typeof choice.earlier === 'boolean' && label(choice.reward) && label(choice.option)
      && signed(choice.province) && signed(choice.progress) && reference(choice.characterReference)
      && (choice.country === null || uint(choice.country)) && (choice.scientist === null || uint(choice.scientist)) && (choice.state === null || uint(choice.state))))) return bad();
  return value as unknown as SpecialProjectDetails;
}

const pendingEffects = (value: unknown) => object(value) && typeof value.reward === 'string' && uint(value.ordinal) && Array.isArray(value.options)
  && value.options.every(option => object(option) && typeof option.token === 'string' && effects(option.effects));

export function validateProjectEffects(value: unknown): SpecialProjectEffects {
  if (!object(value) || !integer(value.hour) || !(value.readHour === null || readBy(value.readHour, value.hour)) || !(value.output === null || effects(value.output))
    || !(value.pending === null || pendingEffects(value.pending)) || (value.readHour === null && (value.output !== null || value.pending !== null))
    || !nullable(value.unavailable) || !strings(value.warnings)) throw new Error('Invalid special-project effects');
  return value as unknown as SpecialProjectEffects;
}

export function effectsRead(hour: number, project: ProjectRuntimeDetails, reply: SpecialProjectEffects): string | null {
  if (reply.readHour === null) return reply.unavailable ?? 'Effects were not read for this project.';
  if (reply.readHour === project.readHour) return null;
  return reply.hour === hour ? 'Effects were read from another reading of this project.' : 'Loading effects…';
}

export function optionEffects(project: ProjectRuntimeDetails, reply: SpecialProjectEffects): ProjectEffects[] | string {
  const asked = project.pending, shown = reply.pending;
  if (!asked) return [];
  if (!shown) return reply.unavailable ?? 'Effects of the pending reward were not read.';
  const same = shown.reward === asked.label.token && shown.ordinal === asked.ordinal && shown.options.length === asked.options.length
    && shown.options.every((option, at) => option.token === asked.options[at].label.token);
  return same ? shown.options.map(option => option.effects) : 'These effects are of another reward than the one pending.';
}

const heading = (value: unknown) => value === null || validGameRuns(value);

export function validateProjectIteration(value: unknown, hour: number): SpecialProjectIteration {
  if (!object(value) || value.hour !== hour || !heading(value.heading)
    || !Array.isArray(value.sections) || !value.sections.every(section => object(section) && heading(section.heading) && validGameRuns(section.runs))
    || (value.heading !== null && value.sections.length === 0)
    || typeof value.noChanges !== 'boolean' || (value.noChanges && (value.heading !== null || value.sections.length > 0))
    || !counts(value.refusals) || !nullable(value.unavailable) || !strings(value.warnings)) throw new Error('Invalid special-project iteration');
  return value as unknown as SpecialProjectIteration;
}

const number = (value: string) => researchNumber(BigInt(value));
const read = (hour: number | null) => hour === null ? '' : ` · read ${dayLabel(hour)}`;

export const scientistState = (scientist: ScientistDetails) => scientist.active === null ? 'State unavailable' : scientist.active === 0 ? 'Unassigned' : 'Assigned';

export function detailTexts(details: SpecialProjectDetails): Map<string, string> {
  const out = new Map<string, string>();
  for (const scientist of details.scientists ?? []) {
    const at = `s:${scientist.character}`;
    out.set(`${at}:state`, scientistState(scientist));
    out.set(`${at}:injury`, `${scientist.injuryDays === null ? 'Injury recovery unavailable.' : scientist.injuryDays > 0 ? `${scientist.injuryDays} injury recovery days` : 'No injury recovery pending'}${read(scientist.injuryHour)}`);
    out.set(`${at}:visibility`, `${scientist.visible === null ? 'Roster visibility unavailable' : scientist.visible ? 'Visible in scientist roster' : 'Hidden from scientist roster'}${read(scientist.visibilityHour)}`);
    scientist.skills?.forEach((skill, index) => out.set(`${at}:skill:${index}`, ` · Level ${skill.level} · ${number(skill.experienceRaw)} experience`));
  }
  details.breakthroughs?.forEach((progress, index) => out.set(`b:${index}`, `: ${progress.points / 10000} breakthrough progress${read(progress.readHour)}`));
  for (const program of details.programs ?? []) {
    const at = `p:${program.id}`;
    out.set(`${at}:read`, `Facility in province ${program.province}${read(program.readHour)}`);
    if (program.facility) {
      out.set(`${at}:speed`, `Research speed: ${program.facility.speedRaw === null ? 'Unavailable' : `${number(program.facility.speedRaw)}×`}`);
      program.facility.sources.forEach((source, index) => out.set(`${at}:source:${index}`,
        `: ${number(source.valueRaw)}${source.unit === 'factor' ? '×' : source.unit === 'bonus' ? ' research bonus' : ''}`));
    }
    program.assignments?.forEach(assignment => out.set(`${at}:assignment:${assignment.ordinal}`,
      assignment.timed !== 0 ? ` · Delayed unassignment: ${assignment.target - assignment.current} days remaining (${assignment.current} / ${assignment.target})` : ''));
  }
  for (const project of details.projects ?? []) {
    const at = `j:${project.owner}:${project.id}`;
    out.set(`${at}:read`, read(project.readHour));
    if (project.seed !== null) out.set(`${at}:seed`, String(project.seed));
    project.variables?.forEach((variable, index) => out.set(`${at}:var:${index}`, number(variable.valueRaw)));
  }
  return out;
}
