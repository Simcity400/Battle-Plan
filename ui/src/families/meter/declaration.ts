import type { IconName } from '../../components/Icons';
import type { OptionField } from '../../dashboard/options';
import type {
  AuxProductSpec,
  CountryPolicy,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import type { MeterProjection } from './model.ts';

export interface MeterWidgetDeclaration<TProjectionOptions = Record<string, never>, TAux = unknown> {
  readonly family: 'meters';
  readonly kind: string;
  readonly label: string;
  readonly subject: string;
  readonly emptyCampaign: string;
  readonly icon?: IconName;
  readonly className?: string;
  readonly frameClassName?: string;
  readonly optionFields?: readonly OptionField[];
  readonly countries: CountryPolicy;
  readonly aux?: AuxProductSpec<TAux>;
  readonly sources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan;
  readonly projection: MeterProjection<TProjectionOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export type AnyMeterWidgetDeclaration = MeterWidgetDeclaration<any, any>;
