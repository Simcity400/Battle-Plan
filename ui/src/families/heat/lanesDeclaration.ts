import type { IconName } from '../../components/Icons';
import type { OptionField } from '../../dashboard/options';
import type { FamilyTheme } from '../contract.ts';
import type {
  AuxProductSpec,
  CountryPolicy,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import type { HeatLaneRow, HeatLanesProjection } from './lanesModel.ts';

export interface HeatLook {
  readonly theme: FamilyTheme;
}

export interface LaneActivation {
  readonly row: HeatLaneRow;
  readonly rowIndex: number;
  readonly hour: number;
}

export interface HeatLanesWidgetDeclaration<TProjectionOptions = Record<string, never>, TAux = unknown> {
  readonly family: 'heat-lanes';
  readonly kind: string;
  readonly view?: string;
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
  readonly projection: HeatLanesProjection<TProjectionOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>, look: HeatLook) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export type AnyHeatLanesWidgetDeclaration = HeatLanesWidgetDeclaration<any, any>;

export function isHeatLanesWidgetDeclaration(value: unknown): value is AnyHeatLanesWidgetDeclaration {
  return !!value
    && typeof value === 'object'
    && (value as { family?: unknown }).family === 'heat-lanes'
    && 'sources' in value
    && 'projection' in value;
}
