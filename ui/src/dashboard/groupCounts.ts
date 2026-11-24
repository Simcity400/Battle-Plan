import type { BranchToHourSourceRead } from '../families/plan.ts';
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

export function groupCountsProof(data: unknown, _requested: readonly [number, number], hour: number): readonly [number, number] | null {
  const reply = data as GroupCountsReply;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= hour && hour <= last ? [reply.hour, last] : null;
}

export function groupCountsRead(branchId: string, of: CountedGroup): BranchToHourSourceRead {
  return {
    shape: 'branch-to-hour',
    coverage: { mode: 'population', form: 'all' },
    product: 'group-counts',
    followQuery: null,
    params: { branchId, of },
    buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
    provenInterval: groupCountsProof,
    dynamicKeys: ['hour'],
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
