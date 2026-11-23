import type { IconName } from '../../components/Icons';
import type { Channel } from '../../dashboard/follow/facts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { BoundWindow, GatedHour } from '../../dashboard/regionTime';

export type GatedSpan = BoundWindow;
import type { DeclaredProduct } from '../../dashboard/request';
import type { CountryLooksMap } from './looks.ts';
import type { TableColumnSpec, TableProjection } from './model.ts';
import type { TableBase, TableSettings, TableSortDirection } from './settings.ts';

export type HourRanges = readonly (readonly [number, number])[];

export interface DeclaredCountryFilter {
  readonly passing: Readonly<Record<string, HourRanges>> | null;
  readonly unrecorded: readonly (readonly [number, number, string])[];
  readonly bounds: readonly number[];
}

export interface DeclaredFactionRun {
  readonly id: string;
  readonly tag: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly names: readonly { readonly firstHour: number; readonly lastHour: number; readonly name: string | null }[];
  readonly fallbackName: string | null;
}

export interface DeclaredCountryState {
  readonly all: boolean;
  readonly tags: readonly string[];
  readonly chosenTags: readonly string[];
  readonly every: readonly string[];
  readonly top: number | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly membershipRuns: readonly { readonly tag: string; readonly firstHour: number; readonly lastHour: number }[] | null;
  readonly filter: DeclaredCountryFilter;
  readonly labels: CountryLooksMap;
  readonly factions: readonly DeclaredFactionRun[] | null;
  readonly rankedAt: (hour: number) => readonly string[];
  readonly reason: string | null;
  readonly known: (first: number, last: number) => boolean;
  readonly unobservedIn: (first: number, last: number) => boolean;
}

export interface DeclaredTableContext<TAux = unknown> {
  readonly track: BoundTrack;
  readonly branch: BoundTrack['branch'];
  readonly widget: WidgetSpec;
  readonly options: Readonly<Record<string, OptionValue>>;
  readonly following: boolean;
  readonly aux: TAux | undefined;
  readonly countryTag: string | null;
  readonly countryName: string;
  readonly countries: DeclaredCountryState;
  readonly settings: TableSettings;
  readonly gateSpan: (first: number, last: number) => GatedSpan | null;
}

export type CountryPolicy =
  | { readonly mode: 'none' }
  | { readonly mode: 'single'; readonly choose?: string }
  | { readonly mode: 'filter'; readonly labels?: boolean; readonly colours?: boolean }
  | {
      readonly mode: 'multi';
      readonly labels?: boolean;
      readonly factions?: boolean;
      readonly allowSlots?: boolean;
      readonly allowSingleSlot?: boolean;
      readonly everyOnTop?: boolean;
      readonly allTagsForLabels?: boolean;
      readonly requireGroupsWhen?: (widget: WidgetSpec, options: Readonly<Record<string, OptionValue>>) => boolean;
    };

export interface AuxProductSpec<TAux = unknown> {
  readonly product: DeclaredProduct;
  readonly params: (track: BoundTrack, options: Readonly<Record<string, OptionValue>>) => Record<string, unknown> | null;
  readonly validate: (raw: unknown) => TAux;
  readonly when?: (options: Readonly<Record<string, OptionValue>>) => boolean;
}

export interface DeclaredFollowItem {
  readonly name: string;
  readonly query: Record<string, unknown>;
  readonly channel?: Channel;
  readonly product?: string;
}

export type DeclaredTableSource<TAux = unknown> =
  | {
      readonly kind: 'panel';
      readonly product: DeclaredProduct;
      readonly params: (ctx: DeclaredTableContext<TAux>) => Record<string, unknown> | null;
      readonly validate: (raw: unknown) => unknown;
    }
  | {
      readonly kind: 'head';
      readonly product: DeclaredProduct;
      readonly timeShape: 'branch-to-hour' | 'trailing-window';
      readonly windowHours?: number;
      readonly followQuery: (ctx: DeclaredTableContext<TAux>) => Record<string, unknown> | null;
      readonly productParams: (ctx: DeclaredTableContext<TAux>, span: GatedSpan) => Record<string, unknown> | null;
      readonly dynamicKeys?: readonly string[];
      readonly validate: (raw: unknown) => unknown;
    }
  | {
      readonly kind: 'multi';
      readonly follow: (ctx: DeclaredTableContext<TAux>) => {
        readonly subs: readonly DeclaredFollowItem[];
        readonly unavailable?: string | null;
      };
      readonly exactProduct: DeclaredProduct;
      readonly exactParams: (ctx: DeclaredTableContext<TAux>, hour: GatedHour, mode: 'static' | 'exact') => Record<string, unknown> | null;
      readonly dynamicKeys?: readonly string[];
      readonly validate: (raw: unknown, ctx: DeclaredTableContext<TAux>) => unknown;
    };

export type TableSettingsPolicy<TAux = unknown> =
  | { readonly kind: 'fixed'; readonly base: TableBase }
  | { readonly kind: 'table'; readonly key?: string; readonly base: TableBase }
  | { readonly kind: 'options'; readonly defaultSort: { readonly column: string; readonly direction: TableSortDirection } }
  | {
      readonly kind: 'custom';
      readonly resolve: (ctx: Omit<DeclaredTableContext<TAux>, 'settings'>, columns: readonly TableColumnSpec[]) => TableSettings;
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
  readonly source: DeclaredTableSource<TAux>;
  readonly projection: TableProjection<TProjectionOptions>;
  readonly columns: readonly TableColumnSpec[] | ((ctx: Omit<DeclaredTableContext<TAux>, 'settings'>) => readonly TableColumnSpec[]);
  readonly settings: TableSettingsPolicy<TAux>;
  readonly projectionOptions: (ctx: DeclaredTableContext<TAux>) => TProjectionOptions;
  readonly unavailable?: (ctx: DeclaredTableContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: DeclaredTableContext<TAux>, followRefused: string | null) => readonly string[];
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export type AnyTableWidgetDeclaration = TableWidgetDeclaration<any, any>;

export function isTableWidgetDeclaration(value: unknown): value is AnyTableWidgetDeclaration {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'table' && 'source' in value && 'projection' in value;
}
