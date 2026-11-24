import type { IconName } from '../../components/Icons';
import type {
  AuxProductSpec,
  CountryPolicy,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import type { CardProjection } from './model.ts';

export interface CardWidgetDeclaration<TProjectionOptions = Record<string, never>, TAux = unknown> {
  readonly family: 'card';
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
  readonly projection: CardProjection<TProjectionOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export type AnyCardWidgetDeclaration = CardWidgetDeclaration<any, any>;

export function isCardWidgetDeclaration(value: unknown): value is AnyCardWidgetDeclaration {
  return !!value && typeof value === 'object'
    && (value as { family?: unknown }).family === 'card'
    && 'sources' in value
    && 'projection' in value;
}
