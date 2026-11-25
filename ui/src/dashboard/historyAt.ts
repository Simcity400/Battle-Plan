import type { PointSourceRead, BoundedReadView } from '../families/plan.ts';
import type { CountryGroupReply, CountryGroupsAtReply, SubjectReply, SubjectsAtReply } from '../generated/dashboard';

function validateHistoryAt(field: 'groups' | 'subjects') {
  return (data: unknown) => {
    const reply = data as Record<string, unknown> | null;
    const ok = reply != null && typeof reply === 'object' && Number.isInteger(reply.hour)
      && (reply.nextChange === null || (Number.isInteger(reply.nextChange) && (reply.nextChange as number) > (reply.hour as number)))
      && reply[field] != null && typeof reply[field] === 'object';
    if (!ok) throw new Error(`The ${field === 'groups' ? 'country groups' : 'subjects'} reply is malformed.`);
    return data;
  };
}

export function historyAtRead(product: 'country-groups-at' | 'subjects-at', branchId: string): PointSourceRead {
  return {
    shape: 'point',
    channel: 'panel',
    coverage: { mode: 'population', form: 'all' },
    product,
    followQuery: { panel: product, branchId },
    params: { branchId },
    validate: validateHistoryAt(product === 'country-groups-at' ? 'groups' : 'subjects'),
  };
}

interface Run { readonly firstHour: number; readonly lastHour: number }

function held<T extends Run>(runs: readonly T[], read: number, hour: number): T[] {
  return runs.map(run => (run.lastHour === read ? { ...run, lastHour: hour } : run));
}

function groupsThrough(groups: CountryGroupReply, read: number, hour: number): CountryGroupReply {
  return {
    ...groups,
    humans: held(groups.humans, read, hour),
    factions: held(groups.factions, read, hour).map(faction => ({
      ...faction,
      names: held(faction.names, read, hour),
      members: held(faction.members, read, hour),
      leaders: held(faction.leaders, read, hour),
      incomplete: held(faction.incomplete, read, hour),
    })),
    wars: held(groups.wars, read, hour),
    countryIdentities: groups.countryIdentities.map(history => ({ ...history, identities: held(history.identities, read, hour) })),
    unobserved: {
      ...groups.unobserved,
      humans: held(groups.unobserved.humans, read, hour),
      factions: held(groups.unobserved.factions, read, hour),
      wars: held(groups.unobserved.wars, read, hour),
    },
  };
}

function subjectsThrough(subjects: SubjectReply, read: number, hour: number): SubjectReply {
  return {
    ...subjects,
    runs: held(subjects.runs, read, hour).map(run => ({ ...run, steps: held(run.steps, read, hour) })),
    unobserved: held(subjects.unobserved, read, hour),
  };
}

const shown = new WeakMap<object, { readonly hour: number; readonly value: unknown }>();

function atShownHour<T>(reply: object, read: number, hour: number, value: T, through: (value: T, read: number, hour: number) => T): T {
  if (hour <= read) return value;
  const kept = shown.get(reply);
  if (kept?.hour === hour) return kept.value as T;
  const made = through(value, read, hour);
  shown.set(reply, { hour, value: made });
  return made;
}

export function groupsAt(readView: BoundedReadView, name: string): { readonly groups: CountryGroupReply; readonly nextChange: number } | undefined {
  const reply = readView.panel<CountryGroupsAtReply>(name);
  return reply ? { groups: atShownHour(reply, reply.hour, readView.hour, reply.groups, groupsThrough), nextChange: reply.nextChange ?? Infinity } : undefined;
}

export function subjectsAt(readView: BoundedReadView, name: string): { readonly subjects: SubjectReply; readonly nextChange: number } | undefined {
  const reply = readView.panel<SubjectsAtReply>(name);
  return reply ? { subjects: atShownHour(reply, reply.hour, readView.hour, reply.subjects, subjectsThrough), nextChange: reply.nextChange ?? Infinity } : undefined;
}
