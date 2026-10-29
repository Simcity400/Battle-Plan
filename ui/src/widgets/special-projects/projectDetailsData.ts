import type { SpecialProjectDetails } from '../../generated/dashboard';
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const integer = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
const uint = (value: unknown) => integer(value) && value >= 0 && value <= 0xffffffff;
const signed = (value: unknown) => integer(value) && value >= -0x80000000 && value <= 0x7fffffff;
const nullable = (value: unknown) => value === null || typeof value === 'string';
const raw = (value: unknown) => typeof value === 'string' && /^-?(0|[1-9][0-9]{0,18})$/.test(value) && BigInt(value) >= -(1n << 63n) && BigInt(value) < (1n << 63n);
const label = (value: unknown) => object(value) && typeof value.token === 'string' && nullable(value.name);
const person = (value: unknown) => object(value) && (value.assigned === null || typeof value.assigned === 'boolean') && (value.id === null || uint(value.id)) && nullable(value.name) && nullable(value.portrait) && nullable(value.missing);
const effects = (value: unknown) => object(value) && typeof value.text === 'string' && object(value.refusals) && Object.values(value.refusals).every(count => integer(count) && count > 0);
const byte = (value: unknown) => integer(value) && value >= 0 && value <= 255;
const facility = (value: unknown) => object(value) && (value.speedRaw === null || raw(value.speedRaw)) && nullable(value.unavailable)
  && Array.isArray(value.sources) && value.sources.every(source => object(source) && typeof source.kind === 'string' && typeof source.unit === 'string'
    && (source.label === null || label(source.label)) && (source.modifier === null || label(source.modifier)) && uint(source.assignment)
    && typeof source.scientistReference === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(source.scientistReference) && BigInt(source.scientistReference) < 1n << 64n && raw(source.valueRaw));
const option = (value: unknown) => object(value) && label(value.label) && effects(value.effects);
const reward = (value: unknown) => object(value) && label(value.label) && nullable(value.description) && nullable(value.unavailable) && Array.isArray(value.options) && value.options.every(option);
/** An unavailable roster is null: it is never read as an empty one. */
const roster = (value: unknown): value is unknown[] | null => value === null || Array.isArray(value);
export function validateProjectDetails(value: unknown): SpecialProjectDetails {
  const bad = () => { throw new Error('Invalid special-project details'); };
  if (!object(value) || !integer(value.hour) || !roster(value.scientists) || !roster(value.programs) || !roster(value.projects) || !roster(value.breakthroughs)
    || !Array.isArray(value.unavailable) || !value.unavailable.every(reason => typeof reason === 'string')) return bad();
  for (const scientist of value.scientists ?? []) if (!object(scientist) || !uint(scientist.character) || !person(scientist.person)
    || !(scientist.active === null || byte(scientist.active)) || !(scientist.injuryDays === null || signed(scientist.injuryDays))
    || !(scientist.visible === null || typeof scientist.visible === 'boolean') || !(scientist.visibilityHour === null || integer(scientist.visibilityHour) && scientist.visibilityHour <= value.hour)
    || !object(scientist.stateRefusals) || !Object.values(scientist.stateRefusals).every(count => integer(count) && count > 0)
    || !(scientist.injuryHour === null || integer(scientist.injuryHour) && scientist.injuryHour <= value.hour)
    || !(scientist.skills === null || Array.isArray(scientist.skills) && scientist.skills.every(skill => object(skill) && label(skill.specialization) && signed(skill.level) && raw(skill.experienceRaw)))
    || !(scientist.traits === null || Array.isArray(scientist.traits) && scientist.traits.every(label))) return bad();
  for (const progress of value.breakthroughs ?? []) if (!object(progress) || !label(progress.specialization) || !signed(progress.points) || !integer(progress.readHour) || progress.readHour > value.hour) return bad();
  for (const program of value.programs ?? []) if (!object(program) || !uint(program.id) || !uint(program.province) || !integer(program.readHour) || program.readHour > value.hour || !(program.facility === null || facility(program.facility))
    || !(program.assignments === null || Array.isArray(program.assignments) && program.assignments.every((assignment, ordinal) => object(assignment) && assignment.ordinal === ordinal
      && typeof assignment.reference === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(assignment.reference) && BigInt(assignment.reference) < 1n << 64n
      && person(assignment.person) && signed(assignment.target) && signed(assignment.current) && byte(assignment.timed)))) return bad();
  for (const project of value.projects ?? []) if (!object(project) || !uint(project.owner) || !uint(project.id) || !label(project.label) || !integer(project.readHour) || project.readHour > value.hour
    || typeof project.pendingKnown !== 'boolean' || !(project.pending === null || project.pendingKnown && reward(project.pending)) || !(project.seed === null || uint(project.seed))
    || !(project.variables === null || Array.isArray(project.variables) && project.variables.every(variable => object(variable) && typeof variable.name === 'string' && raw(variable.valueRaw)))
    || !(project.output === null || effects(project.output))
    || !(project.choices === null || Array.isArray(project.choices) && project.choices.every(choice => object(choice) && uint(choice.ordinal) && integer(choice.date) && label(choice.reward) && label(choice.option)
      && signed(choice.province) && signed(choice.progress) && typeof choice.characterReference === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(choice.characterReference) && BigInt(choice.characterReference) < 1n << 64n
      && (choice.country === null || uint(choice.country)) && (choice.scientist === null || uint(choice.scientist)) && (choice.state === null || uint(choice.state))))) return bad();
  return value as unknown as SpecialProjectDetails;
}
