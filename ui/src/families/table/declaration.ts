import type { IconName } from '../../components/Icons';
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { BoundWindow } from '../../dashboard/regionTime';
import { resolveHeadSpan } from '../broker.ts';
import {
  holderTagAt,
  nextHoldersChange,
  nextMembershipChange,
  nextRangesChange,
  nextWindowRangesChange,
  type FamilyFactionRun,
  type HolderRun,
  type HourRanges,
} from '../holders.ts';
import type {
  AuxProductSpec,
  CountryPolicy,
  DeclaredCountryFilter,
  DeclaredCountryState,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import type { TableColumnSpec, TableProjection } from './model.ts';
import type { TableBase, TableSettings } from './settings.ts';

export type GatedSpan = BoundWindow;
export type DeclaredFactionRun = FamilyFactionRun;
export {
  holderTagAt,
  nextHoldersChange,
  nextMembershipChange,
  nextRangesChange,
  nextWindowRangesChange,
  resolveHeadSpan,
};
export type {
  AuxProductSpec,
  CountryPolicy,
  DeclaredCountryFilter,
  DeclaredCountryState,
  HolderRun,
  HourRanges,
};

export interface DeclaredTableContext<TAux = unknown> extends FamilyPlanContext<TAux> {
  readonly settings: TableSettings;
}

export type TableSettingsPolicy<TAux = unknown> =
  | { readonly kind: 'fixed'; readonly base: TableBase }
  | { readonly kind: 'table'; readonly key?: string; readonly base: TableBase }
  | {
      readonly kind: 'custom';
      readonly resolve: (ctx: FamilyPlanContext<TAux>, columns: readonly TableColumnSpec[]) => TableSettings;
      readonly onChange: (
        next: TableSettings,
        current: TableSettings,
        ctx: DeclaredTableContext<TAux>,
        setOption: (key: string, value: OptionValue) => void,
      ) => void;
    };

export interface TableWidgetDeclaration<TProjectionOptions = Record<string, never>, TAux = unknown> {
  readonly family: 'table';
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
  readonly projection: TableProjection<TProjectionOptions>;
  readonly columns: readonly TableColumnSpec[] | ((ctx: FamilyPlanContext<TAux>) => readonly TableColumnSpec[]);
  readonly settings: TableSettingsPolicy<TAux>;
  readonly projectionOptions: (ctx: DeclaredTableContext<TAux>) => TProjectionOptions;
  readonly unavailable?: (ctx: DeclaredTableContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: DeclaredTableContext<TAux>, followRefused: string | null) => readonly string[];
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export type AnyTableWidgetDeclaration = TableWidgetDeclaration<any, any>;

export function isTableWidgetDeclaration(value: unknown): value is AnyTableWidgetDeclaration {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'table' && 'sources' in value && 'projection' in value;
}
