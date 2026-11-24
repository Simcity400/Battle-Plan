import type { ChartBucket } from '../charts/chartSummary';
import type { LiveValues } from '../charts/live';
import type { MarkCluster } from '../charts/markSummary';
import type { Run } from '../charts/series';
import type { DashboardSource } from '../dashboard/client';
import type { Channel } from '../dashboard/follow/facts';
import type { FollowedAnswer } from '../dashboard/follow/message';
import type { FailureOrigin } from '../dashboard/follow/page';
import type { FollowQuery } from '../dashboard/follow/pipeline';
import type { BoundTrack, WidgetSpec } from '../dashboard/model';
import type { OptionValue } from '../dashboard/options';
import type { Failure } from '../dashboard/queryError';
import type { CountryHolder } from '../dashboard/reachedCountry';
import type { BoundWindow, GatedHour, Outside } from '../dashboard/regionTime';
import type { DeclaredProduct } from '../dashboard/request';
import type { AuditRow } from '../draw/notifications';
import type {
  ArtFrame as GeneratedArtFrame,
  ArtOrigin as GeneratedArtOrigin,
  ArtProvenance as GeneratedArtProvenance,
  ArtResolution as GeneratedArtResolution,
  CountryFlagSize,
  HourSpan,
  MissingSpan,
} from '../generated/dashboard';
import type { FamilyFactionRun, HolderRun, HourRanges } from './holders.ts';
import type { CountryLooksMap } from './looks.ts';
import type { DatedLogRow, LogTailQuery, TableRowSource } from './table/rowSource.ts';
import type { Value } from './value.ts';

export type FamilyId = string;

declare const DISPLAYED_HOUR: unique symbol;
export type DisplayedHour = number & { readonly [DISPLAYED_HOUR]: 'displayed' };

export type SourceCoverage =
  | { readonly mode: 'keyed'; readonly keys: readonly (string | number)[] }
  | { readonly mode: 'population'; readonly form: 'all' | 'top' | 'least' | 'parts' | 'total'; readonly limit?: number };

export interface PointSourceRead {
  readonly shape: 'point';
  readonly channel: 'panel' | 'followed';
  readonly coverage: SourceCoverage;
  readonly product: DeclaredProduct;
  readonly followQuery: Readonly<Record<string, unknown>> | null;
  readonly exactProduct?: DeclaredProduct;
  readonly params: Readonly<Record<string, unknown>> | null;
  readonly hourParam?: string;
  readonly buildExactParams?: (
    base: Readonly<Record<string, unknown>>,
    hour: GatedHour,
    mode: 'static' | 'exact',
  ) => Readonly<Record<string, unknown>> | null;
  readonly provenInterval?: (data: unknown, hour: number) => readonly [number, number] | null;
  readonly dynamicKeys?: readonly string[];
  readonly validate?: (raw: unknown) => unknown;
}

export interface RangeSourceRead {
  readonly shape: 'range';
  readonly coverage: SourceCoverage;
  readonly product: DeclaredProduct;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly params: Readonly<Record<string, unknown>> | null;
  readonly buildSpanParams?: (
    base: Readonly<Record<string, unknown>>,
    span: BoundWindow,
  ) => Readonly<Record<string, unknown>> | null;
  readonly dynamicKeys?: readonly string[];
  readonly validate?: (raw: unknown) => unknown;
}

export interface BranchToHourSourceRead {
  readonly shape: 'branch-to-hour';
  readonly coverage: SourceCoverage;
  readonly product: DeclaredProduct;
  readonly followQuery: Readonly<Record<string, unknown>> | null;
  readonly params: Readonly<Record<string, unknown>> | null;
  readonly provenInterval?: (data: unknown, requested: readonly [number, number], hour: number) => readonly [number, number] | null;
  readonly buildSpanParams?: (
    base: Readonly<Record<string, unknown>>,
    span: BoundWindow,
  ) => Readonly<Record<string, unknown>> | null;
  readonly dynamicKeys?: readonly string[];
  readonly validate?: (raw: unknown) => unknown;
}

export interface TrailingWindowSourceRead {
  readonly shape: 'trailing-window';
  readonly coverage: SourceCoverage;
  readonly product: DeclaredProduct;
  readonly windowHours: number;
  readonly followQuery: Readonly<Record<string, unknown>> | null;
  readonly params: Readonly<Record<string, unknown>> | null;
  readonly provenInterval?: (data: unknown, requested: readonly [number, number], hour: number) => readonly [number, number] | null;
  readonly buildSpanParams?: (
    base: Readonly<Record<string, unknown>>,
    span: BoundWindow,
  ) => Readonly<Record<string, unknown>> | null;
  readonly dynamicKeys?: readonly string[];
  readonly validate?: (raw: unknown) => unknown;
}

export interface PixelBinnedSourceRead {
  readonly shape: 'pixel-binned';
  readonly coverage: SourceCoverage;
  readonly family: Readonly<Record<string, unknown>>;
  readonly tags: readonly string[];
  readonly within?: ReadonlyMap<string, readonly HourSpan[]>;
  readonly marks?: boolean;
  readonly headQuery?: Readonly<Record<string, unknown>> | null;
  readonly exactProduct?: DeclaredProduct;
  readonly params?: Readonly<Record<string, unknown>> | null;
}

export interface ExactHoverSourceRead {
  readonly shape: 'exact-hover';
  readonly product: DeclaredProduct;
  readonly params: Readonly<Record<string, unknown>> | null;
  readonly hourParam?: string;
  readonly validate?: (raw: unknown) => unknown;
}

export interface PagedRowsSourceRead {
  readonly shape: 'paged-rows';
  readonly tailQuery: LogTailQuery;
  readonly pageSize?: number;
  readonly followQuery: Readonly<Record<string, unknown>> | null;
}

export type SourceTimeShape =
  | PointSourceRead
  | RangeSourceRead
  | BranchToHourSourceRead
  | TrailingWindowSourceRead
  | PixelBinnedSourceRead
  | ExactHoverSourceRead
  | PagedRowsSourceRead;

export interface SourceEntry {
  readonly name: string;
  readonly source: DashboardSource;
  readonly branch: string;
  readonly offset: number;
  readonly read: SourceTimeShape;
}

export interface SourcePlan {
  readonly owner: string;
  readonly entries: readonly SourceEntry[];
  readonly staticEntry?: SourceEntry | null;
  readonly exactEntry?: SourceEntry | null;
  readonly allowEmptyFollow?: boolean;
  readonly unavailable?: string | null;
  readonly refused?: string | null;
}

export interface CompiledFollowSubscription {
  readonly name: string;
  readonly source: DashboardSource;
  readonly branch: string;
  readonly offset: number;
  readonly query: FollowQuery;
  readonly channel: Channel;
  readonly product?: string;
}

export interface BoundedSlice<T> extends Iterable<T> {
  readonly closedLength: number;
  readonly length: number;
  readonly tail?: T;
  at(index: number): T | undefined;
}

export interface BoundedSeries {
  readonly tag: string;
  readonly decimals: number;
  readonly binHours: number;
  readonly from: number;
  readonly buckets: BoundedSlice<ChartBucket | null>;
  readonly runs?: BoundedSlice<Run>;
  readonly straddling: 'none' | 'cut' | 'loading';
  readonly headValue?: Value;
  readonly missing: BoundedSlice<MissingSpan>;
  readonly approximate?: BoundedSlice<{ readonly firstHour: number; readonly lastHour: number }>;
  readonly marks: BoundedSlice<MarkCluster>;
  readonly validInterval: readonly [lo: number, hi: number];
}

export interface BoundedHover<T = unknown> {
  readonly hour: DisplayedHour;
  readonly data: T;
}

export type EntityKind = 'country' | 'faction' | 'leader' | 'design' | 'state' | 'province' | 'region';

export type FieldStatus<T> =
  | { readonly status: 'ready'; readonly value: T }
  | { readonly status: 'pending' }
  | { readonly status: 'absent'; readonly reason: string }
  | { readonly status: 'failed'; readonly reason: string; readonly retryable: boolean };

export interface IdentityResult {
  readonly kind: EntityKind;
  readonly key: string;
  readonly atHour: DisplayedHour;
  readonly holder: FieldStatus<string>;
  readonly tag?: FieldStatus<string>;
  readonly name: FieldStatus<string>;
  readonly colour?: FieldStatus<string>;
  readonly flagChoice?: FieldStatus<string>;
  readonly validInterval: readonly [lo: number, hi: number];
  readonly nextChangeAt: number;
}

export type ArtFlagSize = CountryFlagSize;

export type ArtHandle =
  | {
      readonly kind: 'flag';
      readonly choice: string;
      readonly size: ArtFlagSize;
    }
  | {
      readonly kind: 'sprite';
      readonly key: string;
      readonly frame?: number;
      readonly recorded?: string;
      readonly text?: boolean;
    }
  | {
      readonly kind: 'portrait';
      readonly leader: string;
      readonly hour: number;
    }
  | {
      readonly kind: 'path';
      readonly path: string;
      readonly frame?: number;
      readonly frames?: number;
    }
  | {
      readonly kind: 'url';
      readonly url: string;
      readonly frame?: number;
    };

export type ArtOrigin = GeneratedArtOrigin;
export type ArtFrame = GeneratedArtFrame;
export type ArtProvenance = GeneratedArtProvenance;
export type ArtResolution = GeneratedArtResolution;

export type ArtResult<TBitmap = ImageBitmap> =
  | Extract<ArtResolution, { readonly status: 'noArtExpected' | 'pending' | 'confirmedMissing' | 'refused' }>
  | (Extract<ArtResolution, { readonly status: 'ready' }> & { readonly bitmap?: TBitmap })
  | (Extract<ArtResolution, { readonly status: 'failed' }> & { readonly retry?: () => void });

export type IdentityDemand = (kind: EntityKind, key: string, atHour: DisplayedHour) => void;
export type ArtDemand = (handle: ArtHandle) => void;

export interface IdentityProvider {
  readonly demandOwner?: 'view';
  resolve(kind: EntityKind, key: string, atHour: DisplayedHour): IdentityResult;
}

export interface ArtProvider {
  readonly demandOwner?: 'view';
  resolve(handle: ArtHandle): ArtResult;
}

export interface BoundedReadView {
  readonly hour: DisplayedHour;
  readonly validInterval: readonly [lo: number, hi: number];
  readonly revisions: Readonly<Record<string, number>>;
  panel<T = unknown>(name: string): T | undefined;
  answer(name: string): FollowedAnswer | undefined;
  live(name: string): LiveValues<number | bigint | string> | null | undefined;
  refused(name: string): string | undefined;
  series(name: string): ReadonlyMap<string, BoundedSeries> | undefined;
  hover<T = unknown>(name: string, atHour?: number): BoundedHover<T> | undefined;
  rows<TRow extends DatedLogRow = DatedLogRow>(name: string): TableRowSource<TRow> | undefined;
  who(kind: EntityKind, key: string, atHour?: number): IdentityResult;
  art(handle: ArtHandle): ArtResult;
}

export interface InstalledTransform {
  readonly xFrom: number;
  readonly xTo: number;
  readonly yMin: number;
  readonly yMax: number;
  readonly log?: boolean;
}

export interface InstalledViewport {
  readonly width: number;
  readonly height: number;
  readonly dpr: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly transform?: InstalledTransform;
}

export interface InstalledActionTarget {
  readonly subject: string;
  readonly hour: DisplayedHour;
  readonly toggle?: boolean;
  readonly label?: string;
  readonly meta?: unknown;
}

export interface InstalledAction {
  readonly installedId: number;
  readonly owner: string;
  readonly subject: string;
  readonly hour: DisplayedHour;
  readonly toggle?: boolean;
  readonly label?: string;
  readonly meta?: unknown;
}

export interface InstalledPicture {
  readonly id: number;
  readonly owner: string;
  readonly generation: { readonly seek: number; readonly reset: number; readonly binding: number };
  readonly factRevisions: Readonly<Record<string, number>>;
  readonly validInterval: readonly [lo: number, hi: number];
  readonly through: number;
  readonly hour: DisplayedHour;
  readonly viewport: InstalledViewport;
  readonly optionsRevision: string;
  readonly themeRevision: string;
  readonly actionKeys: ReadonlyMap<string, InstalledActionTarget>;
  readonly accessibleDescription: string | null;
  readonly auditId?: number;
  readonly auditRows?: readonly AuditRow[];
}

export interface FamilyViewportRequest {
  readonly following: boolean;
  readonly width?: number;
  readonly height?: number;
  readonly hoverHour?: number | null;
  readonly staticSpan: readonly [number, number] | null;
}

export interface DeclaredCountryFilter {
  readonly passing: Readonly<Record<string, HourRanges>> | null;
  readonly unrecorded: readonly (readonly [number, number, string])[];
  readonly bounds: readonly number[];
}

export interface DeclaredCountryState {
  readonly all: boolean;
  readonly tags: readonly string[];
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly every: readonly string[];
  readonly top: number | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly membershipRuns: readonly { readonly tag: string; readonly firstHour: number; readonly lastHour: number }[] | null;
  readonly unobserved: HourRanges;
  readonly filter: DeclaredCountryFilter;
  readonly labels: CountryLooksMap;
  readonly factions: readonly FamilyFactionRun[] | null;
  readonly factionUnobserved: HourRanges;
  readonly factionIncomplete: HourRanges;
  readonly rankedAt: (hour: number) => readonly string[];
  readonly reason: string | null;
  readonly known: (first: number, last: number) => boolean;
  readonly unobservedIn: (first: number, last: number) => boolean;
}

export type CountryPolicy =
  | { readonly mode: 'none' }
  | { readonly mode: 'single'; readonly choose?: string }
  | { readonly mode: 'filter'; readonly labels?: boolean; readonly colours?: boolean }
  | {
      readonly mode: 'multi';
      readonly labels?: boolean;
      readonly colours?: boolean;
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

export interface FamilyPlanContext<TAux = unknown> {
  readonly track: BoundTrack;
  readonly branch: BoundTrack['branch'];
  readonly lastRecordedHour: number | null;
  readonly widget: WidgetSpec;
  readonly options: Readonly<Record<string, OptionValue>>;
  readonly following: boolean;
  readonly aux: TAux | undefined;
  readonly countryTag: string | null;
  readonly countryName: string;
  readonly countries: DeclaredCountryState;
  readonly gateSpan: (first: number, last: number) => BoundWindow | null;
  readonly staticSpan: readonly [number, number] | null;
}

export type FamilyBodyStatus =
  | { readonly kind: 'ready' }
  | { readonly kind: 'unbound'; readonly message: string }
  | { readonly kind: 'loading'; readonly label: string }
  | { readonly kind: 'outside'; readonly outside?: Outside | null }
  | { readonly kind: 'not-recorded'; readonly reason: string; readonly detail?: string }
  | { readonly kind: 'refused'; readonly reason: string; readonly technical?: string }
  | { readonly kind: 'failed'; readonly failure: Failure; readonly origin: FailureOrigin; readonly onRetry?: () => void }
  | { readonly kind: 'choose'; readonly message: string }
  | { readonly kind: 'holders'; readonly tag: string | null; readonly holders: readonly CountryHolder[]; readonly pick: (holder: CountryHolder) => void }
  | { readonly kind: 'absent'; readonly message: string; readonly detail?: string }
  | { readonly kind: 'no-members'; readonly reason: string }
  | { readonly kind: 'empty'; readonly message: string; readonly detail?: string; readonly unavailable?: boolean };

export interface FamilyViewSpec<TModel, TOptions, TSelection = FamilyPlanContext> {
  readonly kind: string;
  readonly family: FamilyId;
  sources(selection: TSelection, view: FamilyViewportRequest, options: TOptions): SourcePlan;
  project(readView: BoundedReadView, options: TOptions): TModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}
