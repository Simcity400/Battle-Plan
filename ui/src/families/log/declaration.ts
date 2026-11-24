import type { MenuItem } from '../../components/ContextMenu';
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { FollowQuery } from '../../dashboard/follow/pipeline';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { DashboardSource } from '../../dashboard/client';
import type { Channel } from '../../dashboard/follow/facts';
import type { FamilyProjection, FamilySources } from '../contract.ts';
import { lookAt } from '../looks.ts';
import type { LogDensity, LogLayoutKind, LogModel, LogRowSpec } from './model.ts';

export interface LogCountryLook {
  readonly from: number;
  readonly name: string | null;
  readonly flag?: string;
  readonly flagReason?: string;
  readonly color?: string;
  readonly status?: string;
}

export type LogLooksMap = Readonly<Record<string, readonly LogCountryLook[]>>;

export function logLookAt(looks: LogLooksMap | undefined, tag: string | null | undefined, hour: number): LogCountryLook | null {
  if (!looks || !tag) return null;
  return lookAt(looks[tag], hour) ?? null;
}

export interface LogSourceSubscription {
  readonly name: string;
  readonly source: DashboardSource;
  readonly branch: string;
  readonly offset: number;
  readonly query: FollowQuery;
  readonly channel: Channel;
  readonly product?: string;
}

export type LogProjection<TOptions = unknown> = FamilyProjection<LogModel, TOptions> & {
  readonly family: 'log';
  readonly layout: LogLayoutKind;
};

export interface LogChipOption {
  readonly value: string;
  readonly label: string;
  readonly hint?: string;
  readonly colour?: string;
}

export interface LogChipGroup {
  readonly key: string;
  readonly label: string;
  readonly chips: readonly LogChipOption[];
  readonly selected: readonly string[];
  readonly max: number;
  readonly full?: boolean;
  readonly onToggle: (value: string) => void;
}

export interface LogPickerControl {
  readonly key: string;
  readonly label: string;
  readonly automatic: string;
  readonly choices: readonly { readonly value: string; readonly label: string; readonly icon?: string | null; readonly hint?: string }[];
  readonly selected: readonly string[];
  readonly onToggle: (value: string) => void;
  readonly onReset: () => void;
}

export interface LogControlsSpec {
  readonly chipGroups?: readonly LogChipGroup[];
  readonly pickers?: readonly LogPickerControl[];
}

export interface LogScrollPageRequest {
  readonly product: string;
  readonly params: Record<string, unknown>;
}

export interface LogWidgetDeclaration<TOptions = Record<string, OptionValue>, TStatic = unknown> {
  readonly family: 'log';
  readonly kind: string;
  readonly layout: LogLayoutKind;
  readonly twoLine?: boolean;
  readonly label: string;
  readonly subject: string;
  readonly emptyIcon: string;
  readonly optionFields?: readonly OptionField[];
  readonly projection: LogProjection<TOptions>;
  readonly needsLooks?: boolean;
  readonly tailQuery: (ctx: {
    readonly track: BoundTrack;
    readonly widget: WidgetSpec;
    readonly options: TOptions;
    readonly firstHour: number | null;
    readonly limit: number;
    readonly tags: readonly string[] | null;
  }) => { readonly product: string; readonly params: Record<string, unknown> } | null;
  readonly pageQuery?: (ctx: {
    readonly track: BoundTrack;
    readonly widget: WidgetSpec;
    readonly options: TOptions;
    readonly firstHour: number | null;
    readonly toHour: number;
    readonly cursor: number;
    readonly pageSize: number;
    readonly tags: readonly string[] | null;
  }) => LogScrollPageRequest | null;
  readonly densityOf?: (options: TOptions) => LogDensity;
  readonly monthsOf?: (options: TOptions) => boolean;
  readonly chipsVisibleOf?: (options: TOptions) => boolean;
  readonly buildOptions: (raw: Readonly<Record<string, OptionValue>>, staticData: TStatic, track: BoundTrack | undefined, widget: WidgetSpec) => TOptions;
  readonly rowMenu?: (row: LogRowSpec, ctx: { readonly track: BoundTrack; readonly jump: (hour: number) => void }) => MenuItem[];
  readonly onActivateRow?: (row: LogRowSpec, hour: number) => void;
}

export type AnyLogWidgetDeclaration = LogWidgetDeclaration<any, any>;

export interface LogExactReader {
  (hour: number, limit: number): Promise<FamilySources | null>;
}
