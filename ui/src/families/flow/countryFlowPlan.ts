import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { SourcePlan } from '../contract.ts';
import { groupSelection } from '../groupSelection.ts';
import type { FamilyPlanContext, FamilyViewportRequest } from '../plan.ts';
import type { FlowProduct, FlowProjectionOptions } from './model.ts';
import { FLOW_BODY_GAP, FLOW_CENTRE_LABEL_HEIGHT, FLOW_HEADER_HEIGHT, FLOW_LABEL_MIN, FLOW_LEGEND_HEIGHT, FLOW_NODE_GAP, FLOW_PAD } from './painter.ts';

function flowPerSide(view: FamilyViewportRequest, legend: boolean, centred: boolean): number {
  const inset = (legend ? FLOW_LEGEND_HEIGHT : 0) + FLOW_HEADER_HEIGHT + FLOW_BODY_GAP + (centred ? FLOW_CENTRE_LABEL_HEIGHT : 0) + FLOW_PAD / 2;
  const slots = Math.floor(((view.height ?? 0) - inset + FLOW_NODE_GAP) / (FLOW_LABEL_MIN + FLOW_NODE_GAP));
  return Math.max(1, slots - 1);
}

function flowSubject(ctx: FamilyPlanContext): string | null {
  if (ctx.countryTag) return ctx.countryTag;
  if (groupSelection(ctx.widget.countries)) return null;
  return !ctx.countries.all && ctx.countries.top === null && ctx.countries.tags.length === 1 ? ctx.countries.tags[0]! : null;
}

function flowViewSubject(ctx: FamilyPlanContext): string | null {
  return ctx.options.between === 'between' && !ctx.countryTag ? null : flowSubject(ctx);
}

function flowSelection(ctx: FamilyPlanContext, subject: string | null) {
  const group = subject === null ? groupSelection(ctx.widget.countries) : null;
  const tags = subject !== null || group !== null || ctx.countries.all || ctx.countries.top !== null ? [] : ctx.countries.tags;
  const chosen = subject !== null ? { subject: countryFromKey(subject) } : group !== null ? { group } : { countries: tags.map(countryFromKey) };
  return { tags, params: { branchId: ctx.branch.id, ...chosen, between: ctx.options.between === 'between' } };
}

export function flowProjectionOptions(ctx: FamilyPlanContext, product: FlowProduct): FlowProjectionOptions {
  const { id, revision } = ctx.track.source;
  const params = { ...flowSelection(ctx, flowViewSubject(ctx)).params, sideTotalsOnly: true };
  return { foldRead: { product, sourceId: id, revision, params } };
}

export function countryFlowPicture(ctx: FamilyPlanContext, view: FamilyViewportRequest, product: FlowProduct, legend: boolean): SourcePlan {
  const subject = flowViewSubject(ctx);
  return countryFlowPlan(ctx, product, { limit: flowPerSide(view, legend, subject !== null) }, subject);
}

export function countryFlowPlan(
  ctx: FamilyPlanContext,
  product: FlowProduct,
  extra: Readonly<Record<string, unknown>> = {},
  subject: string | null = flowSubject(ctx),
): SourcePlan {
  const { tags, params: selection } = flowSelection(ctx, subject);
  const top = ctx.countries.top;
  const params = {
    ...selection,
    ...extra,
    ...(subject === null && top !== null ? { top } : {}),
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
