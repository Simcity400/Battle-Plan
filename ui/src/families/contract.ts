import type { CountryLooksMap } from './looks.ts';
import type {
  BoundedReadView,
  CompiledFollowSubscription,
  FamilyId,
} from './plan.ts';

export type {
  ArtDemand,
  ArtHandle,
  ArtProvider,
  ArtResult,
  AuxProductSpec,
  BoundedHover,
  BoundedReadView,
  BoundedSeries,
  BoundedSlice,
  BranchToHourSourceRead,
  CompiledFollowSubscription,
  CountryPolicy,
  DeclaredCountryFilter,
  DeclaredCountryState,
  DisplayedHour,
  EntityKind,
  ExactHoverSourceRead,
  FamilyBodyStatus,
  FamilyId,
  FamilyPlanContext,
  FamilyViewSpec,
  FamilyViewportRequest,
  FieldStatus,
  IdentityDemand,
  IdentityProvider,
  IdentityResult,
  InstalledAction,
  InstalledActionTarget,
  InstalledPicture,
  InstalledTransform,
  InstalledViewport,
  PagedRowsSourceRead,
  PixelBinnedSourceRead,
  PointSourceRead,
  RangeSourceRead,
  SourceCoverage,
  SourceEntry,
  SourcePlan,
  SourceTimeShape,
  TrailingWindowSourceRead,
} from './plan.ts';
export {
  BoundedSeriesCursor,
  boundTiledSeries,
  clipMarks,
  clipMissingSpans,
  clipRuns,
  createBoundedReadView,
  createBoundedSlice,
  cutStraddlingBucket,
  displayedHour,
  emptyReadView,
  emptySlice,
  FutureDataError,
  sliceMarks,
  sliceMissingSpans,
  sliceRuns,
  sliceSpansByHour,
  type BoundSeriesOptions,
  type BoundedReadViewInput,
  type BoundedWorkCounter,
  type SlicedSpanResult,
  type StraddlingCutResult,
  type WitnessedPoint,
} from './boundedView.ts';

export type FamilySourceSubscription = CompiledFollowSubscription;

export interface FamilyTheme {
  surface: string;
  text: string;
  muted: string;
  line: string;
  lineSoft: string;
  accent: string;
  blue: string;
  red: string;
  amber: string;
  green: string;
  grey: string;
  hatch: string;
  hatchEdge?: string;
  font: string;
  fontSize: { small: number; body: number };
  lineWidth: number;
  hatchSpacing: number;
  country?: Readonly<Record<string, string>>;
  countryLooks?: CountryLooksMap;
}

export interface FamilyProjection<TModel, TOptions> {
  readonly kind: string;
  readonly family: FamilyId;
  project(readView: BoundedReadView, options: TOptions): TModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export interface MirrorCell {
  readonly column: string;
  readonly spoken: string;
}

export interface MirrorRow {
  readonly key: string;
  readonly name: string;
  readonly rowIndex: number;
  readonly rank?: number;
  readonly level?: number;
  readonly group?: boolean;
  readonly open?: boolean;
  readonly groupHeader?: boolean;
  readonly cells: readonly MirrorCell[];
}

export interface MirrorModel {
  readonly first: number;
  readonly total: number;
  readonly rows: readonly MirrorRow[];
}
