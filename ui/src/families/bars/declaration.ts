import type { IconName } from '../../components/Icons';
import type { DeclaredProjectionExtra } from '../host/DeclaredFamilyWidget.tsx';
import type {
  AuxProductSpec,
  CountryPolicy,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import type { BarsProjection } from './model.ts';

export interface BarsWidgetDeclaration<TProjectionOptions = Record<string, never>, TAux = unknown> {
  readonly family: 'bars';
  readonly kind: string;
  readonly label: string;
  readonly subject: string;
  readonly emptyCampaign: string;
  readonly icon?: IconName;
  readonly className?: string;
  readonly frameClassName?: string;
  readonly countries: CountryPolicy;
  readonly aux?: AuxProductSpec<TAux>;
  readonly sources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan;
  readonly projection: BarsProjection<TProjectionOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>, extra?: DeclaredProjectionExtra) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export type AnyBarsWidgetDeclaration = BarsWidgetDeclaration<any, any>;

export function isBarsWidgetDeclaration(value: unknown): value is AnyBarsWidgetDeclaration {
  return !!value && typeof value === 'object'
    && (value as { family?: unknown }).family === 'bars'
    && 'sources' in value
    && 'projection' in value;
}
