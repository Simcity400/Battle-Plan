import type { ReactNode } from 'react';
import type { MenuItem } from '../../components/ContextMenu';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import type { AuditRow } from '../../draw/notifications';
import type { ArtResolution } from '../art/types.ts';
import type {
  ArtHandle,
  BoundedReadView,
  DisplayedHour,
  FamilyId,
  FamilyProjection,
  FamilyTheme,
  InstalledAction,
  InstalledActionTarget,
  InstalledPicture,
  MirrorModel,
  SourcePlan,
} from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import type { FamilyAuditVerifier } from './auditBinding.ts';
import type { ListboxMirrorItem } from './mirror.ts';

export type FamilyPointerMode = 'discrete' | 'continuous';
export type FamilyScrollMode = 'none' | 'vertical' | 'horizontal' | 'both';
export type FamilyNavigationKind = 'rows' | 'grid' | 'points' | 'none';
export type FamilyMirrorKind = 'listbox' | 'table' | 'none';
export type FamilySkeletonKind = 'table' | 'chart';

export type FamilyArtDemand =
  | ArtHandle
  | { readonly key: string; readonly frame: number };

export interface FamilyStaticSources {
  readonly panels?: Readonly<Record<string, unknown>>;
  readonly answers?: Readonly<Record<string, FollowedAnswer>>;
  readonly hour: number;
  readonly follows?: boolean;
  readonly provenInterval?: readonly [lo: number, hi: number];
}

export interface ParsedFamilyHit {
  readonly key: string;
  readonly subject?: string;
  readonly hour?: number;
  readonly toggle?: boolean;
  readonly kind?: string;
  readonly inspectToken?: string;
  readonly label?: string;
  readonly meta?: unknown;
}

export interface FamilyViewportState {
  readonly width: number;
  readonly height: number;
  readonly dpr: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly logicalScrollTop: number;
  readonly zoomWindow: readonly [number, number] | null;
}

export interface FamilyViewportChange {
  readonly width: number;
  readonly height: number;
  readonly dpr: number;
  readonly deviceWidth: number;
  readonly window: readonly [number, number] | null;
}

export interface FamilyViewWindow {
  readonly first: number;
  readonly count: number;
}

export interface FamilyScrollExtent {
  readonly width?: number;
  readonly height?: number;
}

export interface FamilyGridMetrics {
  readonly rows: number;
  readonly columns: number;
  readonly cellSize: number;
  readonly gridLeft: number;
  readonly gridTop: number;
  readonly rowLabel?: (row: number) => string;
  readonly columnLabel?: (column: number) => string;
}

export interface FamilyMetaState {
  readonly mode: string;
  readonly reason: string | null;
  readonly failureOrigin: FailureOrigin;
  readonly empty: string | null;
  readonly emptyDetail: string | null;
  readonly emptyKind: 'empty' | 'unavailable';
  readonly summaryText: string | null;
  readonly summaryPill: string | null;
  readonly notes: readonly string[];
  readonly warnings: readonly string[];
}

export interface FamilyBindingContext<TOptions, TExtra = Record<string, never>> {
  readonly kind: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly tiledSeries?: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;
  readonly hovers?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly spriteResolutions?: Readonly<Record<string, ArtResolution>>;
  readonly extent?: readonly [number, number] | null;
  readonly plan?: SourcePlan | null;
  readonly viewport: FamilyViewportState;
  readonly viewWindow: FamilyViewWindow | null;
  readonly theme: FamilyTheme | undefined;
  readonly reducedMotion: boolean;
  readonly identitySnapshot: IdentitySnapshot | null;
  readonly selectedKey: string | null;
  readonly focusedKey: string | null;
  readonly focusInside: boolean;
  readonly audit: boolean;
  readonly auditId: number | undefined;
  readonly pauseFlush: number;
  readonly extra: TExtra;
}

export interface FamilyAuditContext<TModel, TOptions, TExtra = Record<string, never>> {
  readonly projection: FamilyProjection<TModel, TOptions>;
  readonly options: TOptions;
  readonly exactReadView: (hour: number) => Promise<BoundedReadView | null>;
  readonly viewport: FamilyViewportState;
  readonly viewWindow: FamilyViewWindow | null;
  readonly theme: FamilyTheme | undefined;
  readonly extra: TExtra;
}

export interface FamilyContextHit {
  readonly key: string | null;
  readonly subject: string | null;
  readonly label: string | null;
  readonly hour: number;
  readonly kind?: string;
  readonly meta?: unknown;
  readonly installed: InstalledPicture;
}

export interface FamilyGestureContext<TOptions, TExtra = Record<string, never>> {
  readonly installed: InstalledPicture | null;
  readonly fields: Readonly<Record<string, unknown>>;
  readonly viewport: FamilyViewportState;
  readonly extent: readonly [number, number] | null;
  readonly options: TOptions;
  readonly extra: TExtra;
  readonly setZoomWindow: (next: readonly [number, number] | null) => void;
  readonly requestBind: () => void;
}

export interface FamilyGestureHandlers<TOptions, TExtra = Record<string, never>> {
  readonly onWheel?: (event: WheelEvent, point: readonly [number, number], ctx: FamilyGestureContext<TOptions, TExtra>) => boolean | void;
  readonly onDragStart?: (point: readonly [number, number], ctx: FamilyGestureContext<TOptions, TExtra>) => void;
  readonly onDragMove?: (point: readonly [number, number], delta: readonly [number, number], ctx: FamilyGestureContext<TOptions, TExtra>) => void;
  readonly onDragEnd?: (ctx: FamilyGestureContext<TOptions, TExtra>) => void;
  readonly onDoubleClick?: (point: readonly [number, number], ctx: FamilyGestureContext<TOptions, TExtra>) => boolean | void;
}

export interface FamilySlotRenderContext<TOptions, TSlotState, TExtra> {
  readonly meta: FamilyMetaState;
  readonly options: TOptions;
  readonly slotState: TSlotState | undefined;
  readonly extra: TExtra;
  readonly bodyBlocked: boolean;
  readonly announce: (message: string) => void;
  readonly requestBind: () => void;
}

export interface FamilyInstallFieldsContext<TExtra = Record<string, never>> {
  readonly fields: Readonly<Record<string, unknown>>;
  readonly visible: boolean;
  readonly frameNode: HTMLElement | null;
  readonly bodyNode: HTMLElement;
  readonly installed: InstalledPicture | null;
  readonly extra: TExtra;
  readonly updateAudit: (id: number, patch: (current: unknown) => unknown) => void;
}

export interface FamilyHostAdapter<
  TModel = unknown,
  TOptions = unknown,
  TBinding = unknown,
  TSlotState = undefined,
  TExtra = Record<string, never>,
> {
  readonly family: FamilyId;
  readonly skeleton?: FamilySkeletonKind;
  readonly pointerMode?: FamilyPointerMode;
  readonly scroll?: FamilyScrollMode;
  readonly navigation: FamilyNavigationKind;
  readonly mirror?: FamilyMirrorKind;
  readonly focusTarget?: 'body' | 'mirror-rows';
  readonly bodyRole?: string;
  readonly auditOwnerId?: number;
  readonly defaultSize?: readonly [width: number, height: number];
  readonly seriesPlotInsetPx?: number;
  readonly classNames?: {
    readonly frame?: string;
    readonly body?: string;
    readonly canvas?: string;
    readonly extent?: string;
    readonly mirror?: string;
  };
  readonly buildBinding: (ctx: FamilyBindingContext<TOptions, TExtra>) => TBinding | null;
  readonly computeViewWindow?: (viewport: FamilyViewportState, extra: TExtra) => FamilyViewWindow | null;
  readonly parseHit?: (hitKey: string | null, installed: InstalledPicture | null) => ParsedFamilyHit | null;
  readonly inspectToken?: (key: string) => string;
  readonly buildActionKeys?: (fields: Readonly<Record<string, unknown>>, hour: DisplayedHour) => ReadonlyMap<string, InstalledActionTarget>;
  readonly resolveHitAction?: (hit: ParsedFamilyHit, installed: InstalledPicture, fields: Readonly<Record<string, unknown>>) => InstalledAction | null;
  readonly getScrollExtent?: (fields: Readonly<Record<string, unknown>>, viewport: FamilyViewportState, extra: TExtra) => FamilyScrollExtent | null;
  readonly getRowStride?: (fields: Readonly<Record<string, unknown>>, extra: TExtra) => number;
  readonly getPageStep?: (fields: Readonly<Record<string, unknown>>, viewport: FamilyViewportState, extra: TExtra) => number;
  readonly getGridMetrics?: (fields: Readonly<Record<string, unknown>>) => FamilyGridMetrics | null;
  readonly parseGridKey?: (key: string | null) => { readonly row: number; readonly column: number } | null;
  readonly formatGridKey?: (row: number, column: number) => string;
  readonly fallbackToFirstOnActivate?: boolean;
  readonly toListboxItems?: (mirror: MirrorModel, selectedKey: string | null) => readonly ListboxMirrorItem[];
  readonly deriveAuditRows?: (model: TModel, hour: number) => readonly AuditRow[];
  readonly auditOrdered?: boolean;
  readonly createAudit?: (ctx: FamilyAuditContext<TModel, TOptions, TExtra>) => {
    readonly cacheKey: readonly unknown[];
    readonly snapshot: unknown;
    readonly verify: FamilyAuditVerifier<unknown>;
  } | null;
  readonly gestures?: FamilyGestureHandlers<TOptions, TExtra>;
  readonly onInstallFields?: (ctx: FamilyInstallFieldsContext<TExtra>) => void;
  readonly extractSlotState?: (
    fields: Readonly<Record<string, unknown>> | undefined,
    viewport: FamilyViewportState,
    meta: FamilyMetaState,
    extra: TExtra,
    prev: TSlotState | undefined,
  ) => TSlotState | undefined;
  readonly renderHeaderSlot?: (ctx: FamilySlotRenderContext<TOptions, TSlotState, TExtra>) => ReactNode;
  readonly renderBeforeBody?: (ctx: FamilySlotRenderContext<TOptions, TSlotState, TExtra>) => ReactNode;
  readonly renderAfterBody?: (ctx: FamilySlotRenderContext<TOptions, TSlotState, TExtra>) => ReactNode;
  readonly renderMirrorHead?: (ctx: FamilySlotRenderContext<TOptions, TSlotState, TExtra>) => ReactNode;
  readonly frameAttributes?: (extra: TExtra) => Readonly<Record<string, string | undefined>>;
  readonly bodyAttributes?: (extra: TExtra) => Readonly<Record<string, string | undefined>>;
  readonly contextMenuItems?: (hit: FamilyContextHit, extra: TExtra) => MenuItem[];
}
