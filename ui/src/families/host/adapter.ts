import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import type { FamilyAuditRow } from '../auditRow.ts';
import type { ArtResolution } from '../art/types.ts';
import type {
  ArtHandle,
  DisplayedHour,
  FamilyId,
  FamilyTheme,
  InstalledAction,
  InstalledActionTarget,
  InstalledPicture,
  SourcePlan,
} from '../contract.ts';
import type { OptionValue } from '../../dashboard/options';
import type { IdentitySnapshot } from '../identity/types.ts';
import type { FamilyPlanContext } from '../plan.ts';
import { normalizeView } from '../manifest.ts';
import type { FamilyNavTuple } from './keyboard.ts';
import { parseDefaultRowHit } from './pointer.ts';

const navActionIndexCache = new WeakMap<readonly FamilyNavTuple[], Map<string, FamilyNavTuple>>();

export function lazyActionKeys(
  nav: readonly FamilyNavTuple[],
  hour: DisplayedHour,
): ReadonlyMap<string, InstalledActionTarget> {
  let materialized: Map<string, InstalledActionTarget> | null = null;
  const toTarget = (entry: FamilyNavTuple): InstalledActionTarget => ({
    subject: entry[0],
    hour,
    ...(entry[3] >= 0 ? { toggle: true } : {}),
    label: entry[1],
    meta: entry[5],
  });
  const index = () => {
    let idx = navActionIndexCache.get(nav);
    if (!idx) {
      idx = new Map();
      for (let i = 0; i < nav.length; i++) {
        const e = nav[i]!;
        idx.set(e[0], e);
      }
      navActionIndexCache.set(nav, idx);
    }
    return idx;
  };
  const ensureAll = () => {
    if (!materialized) {
      materialized = new Map();
      for (let i = 0; i < nav.length; i++) {
        const e = nav[i]!;
        materialized.set(e[0], toTarget(e));
      }
    }
    return materialized;
  };
  return {
    get size() { return nav.length; },
    has(key: string) { return index().has(key); },
    get(key: string) {
      const entry = index().get(key);
      return entry ? toTarget(entry) : undefined;
    },
    forEach(cb, thisArg) { ensureAll().forEach(cb, thisArg); },
    entries() { return ensureAll().entries(); },
    keys() { return ensureAll().keys(); },
    values() { return ensureAll().values(); },
    [Symbol.iterator]() { return ensureAll()[Symbol.iterator](); },
  };
}

export type FamilyPointerMode = 'discrete' | 'continuous';
export type FamilyScrollMode = 'none' | 'vertical' | 'both';
export type FamilyNavigationKind = 'rows' | 'grid' | 'points';
export type FamilyMirrorKind = 'listbox' | 'table' | 'none';
export type FamilySkeletonKind = 'table' | 'chart';

export type FamilyArtDemand = ArtHandle;

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
  readonly zoomWindow: readonly [number, number] | null;
}

export interface FamilyViewportChange {
  readonly width: number;
  readonly height: number;
  readonly dpr: number;
  readonly window: readonly [number, number] | null;
}

export interface FamilyScrollExtent {
  readonly width?: number;
  readonly height?: number;
}

export interface FamilyGridMetrics {
  readonly rows: number;
  readonly columns: number;
  readonly cellSize: number;
  readonly cellWidth?: number;
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
  readonly summaryPartial: string | null;
  readonly notes: readonly string[];
  readonly warnings: readonly string[];
}

export interface FamilyRowWindow {
  readonly first: number;
  readonly count: number;
}

export interface FamilyBindingContext<TOptions, TSettings = unknown> {
  readonly kind: string;
  readonly view: string;
  readonly options: TOptions;
  readonly settings: TSettings | undefined;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly tiledSeries?: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;
  readonly hovers?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly artResolutions?: Readonly<Record<string, ArtResolution>>;
  readonly extent: readonly [number, number] | null;
  readonly plan: SourcePlan | null;
  readonly viewport: FamilyViewportState;
  readonly theme: FamilyTheme | undefined;
  readonly reducedMotion: boolean;
  readonly identitySnapshot: IdentitySnapshot | null;
  readonly selectedKey: string | null;
  readonly focusedKey: string | null;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly audit: boolean;
  readonly auditId: number | undefined;
  readonly pauseFlush: number;
}

export interface FamilyAuditContext<TSettings = unknown> {
  readonly viewport: FamilyViewportState;
  readonly settings: TSettings | undefined;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly theme: FamilyTheme | undefined;
  readonly fields: Readonly<Record<string, unknown>>;
}

export interface FamilySettingsPolicy<TSettings> {
  readonly resolve: (declaration: unknown, ctx: FamilyPlanContext<unknown> | null, options: Readonly<Record<string, unknown>>) => TSettings;
  readonly store?: (declaration: unknown, next: TSettings, live: TSettings, ctx: FamilyPlanContext<unknown>, setOption: (key: string, value: OptionValue) => void) => void;
  readonly editable?: (declaration: unknown) => boolean;
}

export interface FamilyContextHit {
  readonly key: string | null;
  readonly subject: string | null;
  readonly label: string | null;
  readonly hour: number;
  readonly hitHour: number | null;
  readonly kind?: string;
  readonly meta?: unknown;
  readonly installed: InstalledPicture;
}

export interface FamilyGestureContext {
  readonly fields: Readonly<Record<string, unknown>>;
  readonly viewport: FamilyViewportState;
  readonly extent: readonly [number, number] | null;
  readonly setZoomWindow: (next: readonly [number, number] | null) => void;
}

export interface FamilyGestureHandlers {
  readonly onWheel?: (event: WheelEvent, point: readonly [number, number], ctx: FamilyGestureContext) => boolean;
  readonly onDragStart?: (point: readonly [number, number], ctx: FamilyGestureContext) => void;
  readonly onDragMove?: (point: readonly [number, number], delta: readonly [number, number], ctx: FamilyGestureContext) => void;
  readonly onDragEnd?: (ctx: FamilyGestureContext) => void;
  readonly onDoubleClick?: (point: readonly [number, number], ctx: FamilyGestureContext) => boolean;
}

export interface FamilyBindingIdentity {
  readonly family: FamilyId;
  readonly kind: string;
  readonly view: string;
}

export interface FamilyBaseBinding<TOptions = unknown> extends FamilyBindingIdentity {
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly scrollLeft: number;
  readonly theme: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot;
  readonly artResolutions?: Readonly<Record<string, ArtResolution>>;
  readonly selectedKey: string | null;
  readonly focusedKey: string | null;
  readonly audit: boolean;
  readonly auditId?: number;
  readonly pauseFlush: number;
}

export type FamilyBindingExtras<TBinding> = Omit<TBinding, keyof FamilyBaseBinding>;

export function familyBinding<TOptions, TBinding, TSettings>(
  adapter: FamilyHostAdapter<any, TOptions, TBinding, TSettings>,
  ctx: FamilyBindingContext<TOptions, TSettings>,
): (TBinding & FamilyBaseBinding<TOptions>) | null {
  if (!ctx.theme) return null;
  const extras = adapter.buildBinding ? adapter.buildBinding(ctx) : {};
  if (extras === null) return null;
  const base: FamilyBaseBinding<TOptions> = {
    family: adapter.family,
    kind: ctx.kind,
    view: normalizeView(ctx.view),
    options: ctx.options,
    sources: ctx.sources,
    ...(ctx.staticPanels ? { staticPanels: ctx.staticPanels } : {}),
    ...(ctx.staticAnswers ? { staticAnswers: ctx.staticAnswers } : {}),
    width: ctx.viewport.width,
    height: ctx.viewport.height,
    scrollTop: ctx.viewport.scrollTop,
    scrollLeft: ctx.viewport.scrollLeft,
    theme: ctx.theme,
    ...(ctx.identitySnapshot ? { identitySnapshot: ctx.identitySnapshot } : {}),
    ...(ctx.artResolutions ? { artResolutions: ctx.artResolutions } : {}),
    selectedKey: ctx.selectedKey,
    focusedKey: ctx.focusedKey,
    audit: ctx.audit,
    ...(ctx.auditId !== undefined ? { auditId: ctx.auditId } : {}),
    pauseFlush: ctx.pauseFlush,
  };
  return { ...base, ...extras } as TBinding & FamilyBaseBinding<TOptions>;
}

export function parseFamilyHit(adapter: Pick<FamilyHostAdapter, 'parseHit'>, hitKey: string | null): ParsedFamilyHit | null {
  return (adapter.parseHit ?? parseDefaultRowHit)(hitKey);
}

export function familyInspectToken(adapter: Pick<FamilyHostAdapter, 'inspectToken'>, key: string): string {
  return adapter.inspectToken ? adapter.inspectToken(key) : `row:${key}`;
}

export interface FamilyHostAdapter<TModel = unknown, TOptions = unknown, TBinding = unknown, TSettings = unknown> {
  readonly family: FamilyId;
  readonly skeleton?: FamilySkeletonKind;
  readonly pointerMode?: FamilyPointerMode;
  readonly scroll?: FamilyScrollMode;
  readonly navigation: FamilyNavigationKind;
  readonly mirror?: FamilyMirrorKind;
  readonly mirrorFocus?: 'body' | 'rows';
  readonly tree?: boolean;
  readonly settings?: FamilySettingsPolicy<TSettings>;
  readonly chromeState?: (fields: Readonly<Record<string, unknown>>, previous: unknown) => unknown;
  readonly auditOwnerId?: number;
  readonly defaultSize?: readonly [width: number, height: number];
  readonly buildBinding?: (ctx: FamilyBindingContext<TOptions, TSettings>) => FamilyBindingExtras<TBinding> | null;
  readonly parseHit?: (hitKey: string | null) => ParsedFamilyHit | null;
  readonly inspectToken?: (key: string) => string;
  readonly resolveHitAction?: (hit: ParsedFamilyHit, installed: InstalledPicture, fields: Readonly<Record<string, unknown>>) => InstalledAction | null;
  readonly getScrollExtent?: (fields: Readonly<Record<string, unknown>>) => FamilyScrollExtent | null;
  readonly getGridMetrics?: (fields: Readonly<Record<string, unknown>>) => FamilyGridMetrics | null;
  readonly parseGridKey?: (key: string | null) => { readonly row: number; readonly column: number } | null;
  readonly formatGridKey?: (row: number, column: number) => string;
  readonly deriveAuditRows?: (model: TModel, hour: number, context: FamilyAuditContext<TSettings>) => readonly FamilyAuditRow[];
  readonly auditOrdered?: boolean;
  readonly gestures?: FamilyGestureHandlers;
  readonly exclusiveExpansion?: (fields: Readonly<Record<string, unknown>>) => boolean;
}
