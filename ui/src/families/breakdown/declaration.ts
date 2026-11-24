import type { IconName } from '../../components/Icons';
import type { CountryPolicy, FamilyPlanContext, FamilyViewportRequest, SourcePlan } from '../plan.ts';
import type { BreakdownProjection } from './model.ts';
import { breakdownView, type BreakdownViewMode } from './options.ts';

export interface BreakdownViewOptions {
  readonly view: BreakdownViewMode;
  readonly namesInRing: boolean;
}

export interface BreakdownWidgetSpec<TOptions> {
  readonly kind: string;
  readonly label: string;
  readonly subject: string;
  readonly emptyCampaign: string;
  readonly icon?: IconName;
  readonly countries: CountryPolicy;
  readonly sources: (ctx: FamilyPlanContext, view: FamilyViewportRequest) => SourcePlan;
  readonly projection: BreakdownProjection<TOptions & BreakdownViewOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext) => TOptions;
  readonly jumpToDate?: boolean;
}

export interface BreakdownWidgetDeclaration<TOptions> extends Omit<BreakdownWidgetSpec<TOptions>, 'projectionOptions'> {
  readonly family: 'breakdown';
  readonly projectionOptions: (ctx: FamilyPlanContext) => TOptions & BreakdownViewOptions;
}

export function breakdownDeclaration<TOptions>(spec: BreakdownWidgetSpec<TOptions>): BreakdownWidgetDeclaration<TOptions> {
  return {
    ...spec,
    family: 'breakdown',
    projectionOptions: ctx => ({ ...spec.projectionOptions(ctx), view: breakdownView(ctx.options), namesInRing: ctx.options.namesInRing !== false }),
  };
}
