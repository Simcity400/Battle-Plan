import type { PointSourceRead } from '../families/plan.ts';
import type { GroupCount, GroupCountsReply, CountedGroup } from '../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => value != null && typeof value === 'object';
const nullableText = (value: unknown) => value === null || typeof value === 'string';

export function validateGroupCounts(data: unknown): GroupCountsReply {
  const ok = object(data) && typeof data.branchId === 'string' && (data.of === 'factions' || data.of === 'overlords') && Number.isInteger(data.hour)
    && (data.nextChange === null || (Number.isInteger(data.nextChange) && (data.nextChange as number) > (data.hour as number)))
    && nullableText(data.missing) && Array.isArray(data.warnings)
    && Array.isArray(data.groups) && data.groups.every(group => object(group) && typeof group.key === 'string'
      && (group.count === null ? typeof group.missing === 'string' : Number.isInteger(group.count) && group.missing === null));
  if (!ok) throw new Error('The group counts reply is malformed.');
  return data as GroupCountsReply;
}

export function groupCountsRead(branchId: string, of: CountedGroup): PointSourceRead {
  return {
    shape: 'point',
    channel: 'panel',
    coverage: { mode: 'population', form: 'all' },
    product: 'group-counts',
    followQuery: { panel: 'group-counts', branchId, of },
    params: { branchId, of },
    validate: validateGroupCounts,
  };
}

export interface RankedCounts {
  readonly rank: ReadonlyMap<string, number>;
  readonly count: (key: string) => GroupCount | undefined;
  readonly missing: string | null;
  readonly nextChange: number;
}

export function rankedCounts(reply: GroupCountsReply | undefined): RankedCounts | null {
  if (!reply) return null;
  const byKey = new Map(reply.groups.map(group => [group.key, group]));
  return {
    rank: new Map(reply.groups.map((group, index) => [group.key, index])),
    count: key => byKey.get(key),
    missing: reply.missing,
    nextChange: reply.nextChange ?? Infinity,
  };
}
