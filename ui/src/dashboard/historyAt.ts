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

export function groupsAt(readView: BoundedReadView, name: string): { readonly groups: CountryGroupReply; readonly nextChange: number } | undefined {
  const reply = readView.panel<CountryGroupsAtReply>(name);
  return reply ? { groups: reply.groups, nextChange: reply.nextChange ?? Infinity } : undefined;
}

export function subjectsAt(readView: BoundedReadView, name: string): { readonly subjects: SubjectReply; readonly nextChange: number } | undefined {
  const reply = readView.panel<SubjectsAtReply>(name);
  return reply ? { subjects: reply.subjects, nextChange: reply.nextChange ?? Infinity } : undefined;
}
