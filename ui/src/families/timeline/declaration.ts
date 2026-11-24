import type { IconName } from '../../components/Icons';
import type { OptionField } from '../../dashboard/options';
import type { AuxProductSpec, CountryPolicy, FamilyPlanContext, FamilyViewportRequest, SourcePlan } from '../plan.ts';
import type { TimelineProjection } from './model.ts';

export interface TimelineProjectionExtra {
  readonly extent: readonly [number, number];
  readonly surface: string;
}

export interface TimelineWidgetDeclaration<TProjectionOptions, TAux = unknown> {
  readonly family: 'timeline';
  readonly kind: string;
  readonly label: string;
  readonly subject: string;
  readonly emptyCampaign: string;
  readonly icon?: IconName;
  readonly optionFields?: readonly OptionField[];
  readonly countries: CountryPolicy;
  readonly aux?: AuxProductSpec<TAux>;
  readonly sources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan;
  readonly projection: TimelineProjection<TProjectionOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>, extra: TimelineProjectionExtra) => TProjectionOptions;
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export type AnyTimelineWidgetDeclaration = TimelineWidgetDeclaration<any, any>;
