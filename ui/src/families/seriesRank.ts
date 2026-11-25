import { validateRankReply } from '../charts/race/measureRaceData.ts';
import { countryFromKey } from '../dashboard/countryKey.ts';
import type { GroupSelection } from '../generated/dashboard';
import type { FamilyPlanContext, SourceCoverage, SourceEntry } from './plan.ts';

export function rankSource(
  ctx: Pick<FamilyPlanContext, 'branch' | 'track'>,
  name: string,
  family: Readonly<Record<string, unknown>>,
  coverage: SourceCoverage,
  countries: readonly string[],
  top: number | null = null,
  group: GroupSelection | null = null,
  measures: readonly string[] = [],
): SourceEntry[] {
  if (top === null && !group && !countries.length) return [];
  const params = {
    branchId: ctx.branch.id, family,
    ...(top !== null ? { top } : group ? { group } : { countries: countries.map(countryFromKey) }),
    ...(measures.length ? { measures } : {}),
  };
  return [{
    name, source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset,
    read: {
      shape: 'point', channel: 'panel', coverage, product: 'series-rank',
      followQuery: { panel: 'series-rank', ...params }, params, validate: validateRankReply,
    },
  }];
}
