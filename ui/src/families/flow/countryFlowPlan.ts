import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { GroupSelection } from '../../generated/dashboard';
import type { SourcePlan } from '../contract.ts';
import { groupSelection, groupUnavailable } from '../groupSelection.ts';
import type { FamilyPlanContext, FamilyViewportRequest } from '../plan.ts';

export const FLOW_PER_SIDE = 6;
const FLOW_ROW_PITCH = 24;
const FLOW_CHROME = 70;

export function flowPerSide(view: FamilyViewportRequest): number {
  if (view.height === undefined) return FLOW_PER_SIDE;
  return Math.max(2, Math.min(FLOW_PER_SIDE, Math.floor((view.height - FLOW_CHROME) / FLOW_ROW_PITCH) - 1));
}

export function flowGroup(ctx: FamilyPlanContext): GroupSelection | null {
  return groupSelection(ctx.widget.countries);
}

export function flowUnavailable(ctx: FamilyPlanContext): string | null {
  return groupUnavailable(ctx.widget.countries);
}

export function flowSubject(ctx: FamilyPlanContext): string | null {
  if (ctx.countryTag) return ctx.countryTag;
  if (flowGroup(ctx)) return null;
  return !ctx.countries.all && ctx.countries.top === null && ctx.countries.tags.length === 1 ? ctx.countries.tags[0]! : null;
}

export function countryFlowPlan(
  ctx: FamilyPlanContext,
  product: 'trade-flow' | 'production-licences' | 'volunteer-flow',
  extra: Readonly<Record<string, unknown>> = {},
  subject: string | null = flowSubject(ctx),
): SourcePlan {
  const top = ctx.countries.top;
  const group = subject === null ? flowGroup(ctx) : null;
  const tags = subject !== null || group !== null || ctx.countries.all || top !== null ? [] : ctx.countries.tags;
  const chosen = subject !== null ? { subject: countryFromKey(subject) } : group !== null ? { group } : { countries: tags.map(countryFromKey) };
  const params: Record<string, unknown> = {
    branchId: ctx.branch.id,
    ...chosen,
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
