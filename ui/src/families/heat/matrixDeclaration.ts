import type { IconName } from '../../components/Icons';
import type { OptionField } from '../../dashboard/options';
import type {
  AuxProductSpec,
  CountryPolicy,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import type { HeatMatrixProjection, MatrixAxisEntity } from './matrixModel.ts';

export interface MatrixActivation {
  readonly row: MatrixAxisEntity;
  readonly column: MatrixAxisEntity;
  readonly rowIndex: number;
  readonly columnIndex: number;
  readonly hour: number;
}

export interface HeatMatrixWidgetDeclaration<TProjectionOptions = Record<string, never>, TAux = unknown> {
  readonly family: 'pairs';
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
  readonly projection: HeatMatrixProjection<TProjectionOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export type AnyHeatMatrixWidgetDeclaration = HeatMatrixWidgetDeclaration<any, any>;

export function isHeatMatrixWidgetDeclaration(value: unknown): value is AnyHeatMatrixWidgetDeclaration {
  return !!value
    && typeof value === 'object'
    && (value as { family?: unknown }).family === 'pairs'
    && 'sources' in value
    && 'projection' in value;
}
