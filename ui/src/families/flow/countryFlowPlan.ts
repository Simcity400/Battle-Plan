import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { SourcePlan } from '../contract.ts';
import type { FamilyPlanContext } from '../plan.ts';

export const FLOW_PER_SIDE = 6;

export function flowSubject(ctx: FamilyPlanContext): string | null {
  if (ctx.countryTag) return ctx.countryTag;
  return !ctx.countries.all && ctx.countries.top === null && ctx.countries.tags.length === 1 ? ctx.countries.tags[0]! : null;
}

export function countryFlowPlan(
  ctx: FamilyPlanContext,
  product: 'trade-flow' | 'production-licences' | 'volunteer-flow',
  extra: Readonly<Record<string, unknown>> = {},
  subject: string | null = flowSubject(ctx),
): SourcePlan {
  const top = ctx.countries.top;
  const tags = subject !== null || ctx.countries.all || top !== null ? [] : ctx.countries.tags;
  const params: Record<string, unknown> = {
    branchId: ctx.branch.id,
    ...(subject !== null ? { subject: countryFromKey(subject) } : { countries: tags.map(countryFromKey) }),
    between: ctx.options.between === 'between',
    ...extra,
  };
  return {
    owner: `${product}:${ctx.branch.id}:${JSON.stringify(params)}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: subject !== null ? { mode: 'keyed', keys: [subject] } : tags.length ? { mode: 'keyed', keys: tags } : { mode: 'population', form: 'all' },
        product,
        followQuery: { panel: product, ...params },
        params,
      },
    }],
  };
}
