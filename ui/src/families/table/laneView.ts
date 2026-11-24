import type { LiveValues } from '../../charts/live';
import { NOT_LIVE_CELL, NOT_RECORDED, repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { AuditRow } from '../../draw/notifications';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import { sectionLiveValues, type LiveMap } from '../../draw/views/fields.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import { createBoundedReadView } from '../boundedView.ts';
import type { ArtDemand, ArtProvider, BoundedReadView, EntityKind, IdentityDemand, IdentityProvider } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { buildColumnLayouts, layoutFamilyTable, measureFixedColumnWidths, type FamilyColumnLayout, type FamilyTableLayout } from './layout.ts';
import { nextLookChange, type CountryLooksMap } from '../looks.ts';
import {
  computeColumnMaxima,
  deriveTableAudit,
  deriveTableMirror,
  deriveTableTooltip,
  nextTableChange,
  resolveTableRows,
  resolveTableRowsIncremental,
  updateColumnMaximaIncremental,
  type FlatTableRow,
  type TableColumnSpec,
  type TableModel,
  type TableRow,
} from './model.ts';
import {
  collectVisibleCellIcons,
  computeRowLayout,
  createTablePicture,
  drawFamilyTable,
  glideDurationForSpeed,
  hitFamilyTable,
  rankHoldForSpeed,
  type FamilyTablePicture,
  type RowLayoutGeometry,
} from './painter.ts';
import { ensureTableProjection } from './registry.ts';
import { MEDIA_ROW_HEIGHT, ROW_HEIGHT, rankMarkers, type GlideOffset, type RankMarker, type TableSettings, type TableTheme } from './settings.ts';

export interface FamilyTableBinding<TOptions = unknown> {
  readonly family: 'table';
  readonly kind: string;
  readonly view?: string;
  readonly options: TOptions;
  readonly settings: TableSettings;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly theme?: TableTheme;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly rankWidth?: number;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly expanded?: Readonly<Record<string, boolean>>;
  readonly reducedMotion?: boolean;
  readonly window?: { readonly first: number; readonly count: number };
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyTableBinding(value: unknown): value is FamilyTableBinding {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'table' && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface PhaseTally {
  serves: number;
  projMs: number;
  layoutMs: number;
  paintMs: number;
  fieldsMs: number;
  auditMs: number;
}

type NavTuple = readonly [string, string, number, number, string, unknown];

interface HeldFamilyTable {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  picture: FamilyTablePicture;
  content: string;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | null | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artProvider: ArtProvider;
  demandedArtUrls: string[];
  identityDemandsField: FieldEntry | null;
  widthConfigKey: string;
  sampleSig: string;
  layoutSampleSig: string;
  fixedWidths: Record<string, number>;
  priorRows: readonly FlatTableRow[];
  priorHour: number;
  glide: Map<string, GlideOffset>;
  order?: { sort: string; keys: string[] };
  markers: Map<string, RankMarker>;
  kept: Map<string, { text: string; content: string }>;
  wake?: () => void;

  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStaticPanels: unknown;
  prevStaticAnswers: unknown;
  prevSortKey: string;
  prevExpanded: unknown;
  validFrom: number;
  validUntil: number;
  cachedModel: TableModel | null;
  cachedModelKind: string;
  cachedModelHasTimeBoundaries: boolean;
  cachedModelNextAt: number;
  cachedRows: readonly FlatTableRow[];
  cachedHasGroupHeaders: boolean;
  cachedHasVariableHeights: boolean;
  cachedRowGeomKey: string;
  cachedRowGeom: RowLayoutGeometry | null;
  cachedRowIndex: Map<string, number>;

  layoutCols: readonly TableColumnSpec[] | null;
  layoutSettings: TableSettings | null;
  layoutWidth: number;
  layoutRankShown: boolean;
  layoutRankWidth: number;
  layoutTheme: TableTheme | undefined;
  cachedAllLayouts: FamilyColumnLayout[];
  cachedLayout: FamilyTableLayout | null;
  widthsField: FieldEntry | null;

  metaSig: string;
  metaField: FieldEntry | null;
  navGate: CadenceGate;
  navItems: readonly NavTuple[] | null;
  navField: FieldEntry | null;
  tableSig: string;
  tableField: FieldEntry | null;

  mirrorGate: CadenceGate;
  mirrorField: FieldEntry | null;

  tipGate: CadenceGate;
  tipField: FieldEntry | null;

  focusTipGate: CadenceGate;
  focusTipField: FieldEntry | null;

  flusher: TrailingFlushScheduler;
  markerTimer: ReturnType<typeof setTimeout> | null;
  markerTimerAt: number;

  cachedMaxima: Map<string, number> | null;
  cachedMaximaCols: string;
  rowAuditCache: WeakMap<TableRow, { readonly sig: string; readonly rows: readonly AuditRow[] }>;

  auditId: number;
  auditWindowKey: string;
  auditField: FieldEntry | null;
  phases: PhaseTally;
  phasesField: FieldEntry | null;
}

const MIRROR_CADENCE_MS = 1000;
const TOOLTIP_CADENCE_MS = 250;
const TEXT_SAMPLE_ROWS = 40;

export const liveAdapter = (map: LiveMap | null): LiveValues<number | bigint | string> | null => map && {
  get: (key: string) => {
    const value = map.get(key);
    return typeof value === 'number' || typeof value === 'bigint' ? value : undefined;
  },
  raw: (key: string) => {
    const value = map.get(key);
    return typeof value === 'bigint' ? value : typeof value === 'number' && Number.isSafeInteger(value) ? BigInt(value) : undefined;
  },
  missing: (key: string) => {
    if (!map.has(key)) return NOT_LIVE_CELL;
    const value = map.get(key);
    return typeof value === 'string' && value.length > 0 ? value : NOT_RECORDED;
  },
};

function answerOf(value: LaneValue): FollowedAnswer | undefined {
  return value.kind === 'answer' ? value.answer : undefined;
}

function optionsLooks(options: unknown): CountryLooksMap | undefined {
  if (!options || typeof options !== 'object') return undefined;
  const obj = options as { labels?: CountryLooksMap; looks?: CountryLooksMap };
  return obj.labels ?? obj.looks;
}

function samePictures(prev: readonly (readonly unknown[])[], pictures: readonly Picture<LaneValue>[]): boolean {
  if (prev.length !== pictures.length) return false;
  for (let i = 0; i < pictures.length; i++) {
    const a = prev[i]!;
    const b = pictures[i]!.parts;
    if (a.length !== b.length) return false;
    for (let j = 0; j < b.length; j++) if (a[j] !== b[j]) return false;
  }
  return true;
}

function snapshotPictures(pictures: readonly Picture<LaneValue>[]): readonly (readonly unknown[])[] {
  return pictures.map(picture => picture.parts.slice());
}

export interface ReadViewProviders {
  readonly identity?: IdentityProvider;
  readonly onIdentityDemand?: IdentityDemand;
  readonly art?: ArtProvider;
  readonly onArtDemand?: ArtDemand;
}

export function readViewFromScene(
  scene: Scene,
  binding: FamilyTableBinding,
  hour: number,
  providers?: ReadViewProviders,
): BoundedReadView {
  const liveCache = new Map<string, LiveValues<number | bigint | string> | null>();
  const resolveLive = (name: string): LiveValues<number | bigint | string> | null | undefined => {
    if (liveCache.has(name)) return liveCache.get(name);
    if (binding.staticPanels && Object.hasOwn(binding.staticPanels, name)) {
      const val = repliedCells(binding.staticPanels[name]).read();
      liveCache.set(name, val);
      return val;
    }
    if (Object.hasOwn(binding.sources, name)) {
      const subId = binding.sources[name]!;
      const at = scene.subs.findIndex(sub => sub.id === subId);
      const picture = at < 0 ? undefined : scene.pictures[at];
      const head = picture?.parts[0]?.value;
      if (head && head.kind === 'panel' && head.refused === null) {
        const val = sectionLiveValues(picture!);
        liveCache.set(name, val);
        return val;
      }
    }
    return undefined;
  };
  const revisions: Record<string, number> = {};
  for (const [name, subId] of Object.entries(binding.sources)) {
    const at = scene.subs.findIndex(sub => sub.id === subId);
    const rev = at < 0 ? undefined : scene.pictures[at]?.parts[0]?.version;
    if (rev !== undefined) revisions[name] = rev;
  }
  return createBoundedReadView({
    hour,
    revisions,
    ensureNonLive: () => {
      const answers = new Map<string, FollowedAnswer | undefined>();
      const panels = new Map<string, unknown>();
      const refused = new Map<string, string>();
      if (binding.staticAnswers) {
        for (const [name, answer] of Object.entries(binding.staticAnswers)) answers.set(name, answer);
      }
      if (binding.staticPanels) {
        for (const [name, data] of Object.entries(binding.staticPanels)) panels.set(name, data);
      }
      for (const [name, subId] of Object.entries(binding.sources)) {
        const at = scene.subs.findIndex(sub => sub.id === subId);
        const picture = at < 0 ? undefined : scene.pictures[at];
        const head = picture?.parts[0]?.value;
        if (!head) continue;
        if (head.kind === 'answer') answers.set(name, answerOf(head));
        else if (head.kind === 'panel') {
          if (head.refused === null) panels.set(name, head.data);
          else refused.set(name, head.refused);
        }
      }
      return { panels, answers, refused };
    },
    resolveLive,
    ...(providers?.identity ? { identity: providers.identity } : {}),
    ...(providers?.onIdentityDemand ? { onIdentityDemand: providers.onIdentityDemand } : {}),
    ...(providers?.art ? { art: providers.art } : {}),
    ...(providers?.onArtDemand ? { onArtDemand: providers.onArtDemand } : {}),
  });
}

function sameColumnSpecs(a: readonly TableColumnSpec[] | null, b: readonly TableColumnSpec[]): boolean {
  if (a === b) return true;
  if (!a || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const ca = a[i]!, cb = b[i]!;
    if (
      ca.key !== cb.key
      || ca.label !== cb.label
      || ca.as !== cb.as
      || ca.tone !== cb.tone
      || ca.priority !== cb.priority
      || ca.numeric !== cb.numeric
      || ca.text !== cb.text
      || ca.hideable !== cb.hideable
      || ca.sortable !== cb.sortable
      || ca.hint !== cb.hint
      || ca.format !== cb.format
      || ca.offered.length !== cb.offered.length
      || (ca.widest?.length ?? 0) !== (cb.widest?.length ?? 0)
    ) return false;
    for (let j = 0; j < ca.offered.length; j++) if (ca.offered[j] !== cb.offered[j]) return false;
    if (ca.widest && cb.widest) {
      for (let j = 0; j < ca.widest.length; j++) if (ca.widest[j] !== cb.widest[j]) return false;
    }
  }
  return true;
}

function sameNavItems(prev: readonly NavTuple[] | null, rows: readonly FlatTableRow[]): boolean {
  if (!prev) return false;
  let pos = 0;
  for (let i = 0; i < rows.length; i++) {
    const item = rows[i]!;
    if (item.kind !== 'data') continue;
    const old = prev[pos++];
    if (!old) return false;
    const treeState = item.isTreeGroup ? (item.isOpen ? 1 : 0) : -1;
    if (
      old[0] !== item.key
      || old[1] !== item.name
      || old[2] !== i
      || old[3] !== treeState
      || old[4] !== (item.parent ?? '')
      || old[5] !== (item.row.meta ?? null)
    ) return false;
  }
  return pos === prev.length;
}

function cachedField(held: HeldFamilyTable, name: string, text: string, content: () => string): string {
  const existing = held.kept.get(name);
  if (existing && existing.text === text) return existing.content;
  const next = { text, content: content() };
  held.kept.set(name, next);
  return next.content;
}

function widthConfigOf(columns: readonly TableColumnSpec[], settings: TableSettings, theme: TableTheme | undefined): string {
  return JSON.stringify([
    theme?.font ?? '',
    theme?.fontSize.small ?? 0,
    theme?.fontSize.body ?? 0,
    settings.order,
    columns.map(c => {
      const own = settings.columns[c.key];
      return [c.key, c.label, c.as, own?.show ?? true, own?.as ?? '', own?.width ?? null, c.numeric ? 1 : 0];
    }),
  ]);
}

function textSampleSignature(columns: readonly TableColumnSpec[], rows: readonly FlatTableRow[]): string {
  const limit = Math.min(rows.length, TEXT_SAMPLE_ROWS);
  let sig = '';
  for (const column of columns) {
    if (column.numeric || column.widest?.length) continue;
    let longest = 0;
    for (let i = 0; i < limit; i++) {
      const item = rows[i]!;
      if (item.kind !== 'data') continue;
      const cell = item.row.cells[column.key];
      if (!cell) continue;
      const val = cell.looks ? item.name : cell.value.kind === 'known' ? String(cell.value.value) : '';
      if (val.length > longest) longest = val.length;
    }
    sig += `${column.key}:${longest}|`;
  }
  return sig;
}

function initialTextSamples(columns: readonly FamilyColumnLayout[], rows: readonly FlatTableRow[]): Record<string, string[]> {
  const samples: Record<string, string[]> = {};
  const limit = Math.min(rows.length, TEXT_SAMPLE_ROWS);
  for (const column of columns) {
    if (column.numeric || column.widest?.length) continue;
    const list: string[] = [];
    for (let i = 0; i < limit; i++) {
      const item = rows[i]!;
      if (item.kind !== 'data') continue;
      const cell = item.row.cells[column.key];
      if (!cell) continue;
      const val = cell.looks ? item.name : cell.value.kind === 'known' ? String(cell.value.value) : '';
      if (val) list.push(val);
    }
    samples[column.key] = list;
  }
  return samples;
}

function metaSignature(model: TableModel, hasTree: boolean, rankShown: boolean, rankWidth: number): string {
  return JSON.stringify([
    model.empty ?? null,
    model.emptyDetail ?? null,
    model.emptyKind ?? 'empty',
    model.titleSummary ?? null,
    model.titleSummaryPartial ?? null,
    model.hideHeader === true,
    model.rowHeightMode ?? 'standard',
    model.notes ?? [],
    model.warnings ?? [],
    hasTree,
    model.unsorted === true,
    rankShown,
    rankWidth,
    model.columns.map(c => [c.key, c.label, c.unit ?? '', c.priority, c.as, c.offered, c.text ? 1 : 0, c.hideable === false ? 0 : 1, c.sortable === false ? 0 : 1, c.numeric ? 1 : 0, c.hint ?? '', c.notRecordedReason ?? '', c.cellKind ?? '', c.pinned ? 1 : 0]),
  ]);
}

export interface FamilyTableLaneOptions {
  readonly cadenceTimers?: CadenceTimers;
  readonly clock?: () => number;
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
}

export function familyTableLaneView(
  wake: (widget: number) => void,
  laneOptions?: FamilyTableLaneOptions,
): LaneView {
  return {
    next(scene: Scene, hour: number, state?: Map<string, unknown>): number {
      const bound = scene.model;
      if (!isFamilyTableBinding(bound)) return Infinity;
      const projection = ensureTableProjection(bound.kind, bound.view, () => wake(scene.widget));
      if (projection === 'loading') return hour + 1;
      if (projection === 'unknown') return Infinity;
      let soonest = Math.min(
        nextLookChange(optionsLooks(bound.options), hour),
        nextLookChange(bound.theme?.countryLooks, hour),
      );
      if (projection.nextChange) {
        const readView = readViewFromScene(scene, bound, hour);
        soonest = Math.min(soonest, projection.nextChange(readView, bound.options));
      }
      const held = state?.get('familyTable') as HeldFamilyTable | undefined;
      if (
        held?.cachedModel
        && held.cachedModelHasTimeBoundaries
        && held.cachedModelKind === bound.kind
        && samePictures(held.prevParts, scene.pictures)
        && held.prevOptions === bound.options
        && held.prevStaticPanels === bound.staticPanels
        && held.prevStaticAnswers === bound.staticAnswers
      ) {
        soonest = Math.min(soonest, nextTableChange(held.cachedModel, hour));
      }
      return soonest;
    },
    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get('familyTable') as HeldFamilyTable | undefined;
      if (held) {
        held.flusher.cancel();
        if (held.markerTimer !== null) {
          (laneOptions?.cadenceTimers?.clear ?? clearTimeout)(held.markerTimer);
          held.markerTimer = null;
          held.markerTimerAt = Infinity;
        }
        held.art.dispose();
        state.delete('familyTable');
      }
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyTableBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };
      const projection = ensureTableProjection(bound.kind, bound.view, () => wake(input.widget));
      if (projection === 'loading') {
        return { surfaces: [], fields: {}, motion: false, nextChangeAt: input.hour + 1 };
      }
      if (projection === 'unknown') {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown table declaration: ${bound.kind}` };
      }
      const timed = bound.audit === true;
      const t0 = timed ? performance.now() : 0;

      let held = state.get('familyTable') as HeldFamilyTable | undefined;
      const dpr = surface.dpr || 1;
      const widthPx = Math.max(1, Math.round(bound.width * dpr));
      const heightPx = Math.max(1, Math.round(bound.height * dpr));
      if (!held) {
        const canvas = new OffscreenCanvas(widthPx, heightPx);
        const ctx = canvas.getContext('2d')!;
        const navGate = new CadenceGate(MIRROR_CADENCE_MS);
        const mirrorGate = new CadenceGate(MIRROR_CADENCE_MS);
        const tipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const focusTipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const art = new LaneArtCache({
          maxEntries: laneOptions?.artMaxEntries,
          fetchBitmap: laneOptions?.fetchBitmap,
        });
        state.set('familyTable', held = {
          canvas,
          ctx,
          picture: createTablePicture(),
          content: content(),
          art,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artProvider: createLaneArtProvider(art),
          demandedArtUrls: [],
          identityDemandsField: null,
          widthConfigKey: '',
          sampleSig: '',
          layoutSampleSig: '',
          fixedWidths: {},
          priorRows: [],
          priorHour: -Infinity,
          glide: new Map(),
          markers: new Map(),
          kept: new Map(),
          prevParts: [],
          prevOptions: undefined,
          prevStaticPanels: undefined,
          prevStaticAnswers: undefined,
          prevSortKey: '',
          prevExpanded: undefined,
          validFrom: Infinity,
          validUntil: -Infinity,
          cachedModel: null,
          cachedModelKind: '',
          cachedModelHasTimeBoundaries: false,
          cachedModelNextAt: Infinity,
          cachedRows: [],
          cachedHasGroupHeaders: false,
          cachedHasVariableHeights: false,
          cachedRowGeomKey: '',
          cachedRowGeom: null,
          cachedRowIndex: new Map(),
          layoutCols: null,
          layoutSettings: null,
          layoutWidth: -1,
          layoutRankShown: false,
          layoutRankWidth: -1,
          layoutTheme: undefined,
          cachedAllLayouts: [],
          cachedLayout: null,
          widthsField: null,
          metaSig: '',
          metaField: null,
          navGate,
          navItems: null,
          navField: null,
          tableSig: '',
          tableField: null,
          mirrorGate,
          mirrorField: null,
          tipGate,
          tipField: null,
          focusTipGate,
          focusTipField: null,
          flusher: new TrailingFlushScheduler([navGate, mirrorGate, tipGate, focusTipGate], laneOptions?.cadenceTimers),
          markerTimer: null,
          markerTimerAt: Infinity,
          cachedMaxima: null,
          cachedMaximaCols: '',
          rowAuditCache: new WeakMap(),
          auditId: -1,
          auditWindowKey: '',
          auditField: null,
          phases: { serves: 0, projMs: 0, layoutMs: 0, paintMs: 0, fieldsMs: 0, auditMs: 0 },
          phasesField: null,
        });
        const holder = held;
        art.setOnSettle(() => {
          holder.picture = createTablePicture();
          holder.wake?.();
        });
      }
      held.wake = () => wake(input.widget);
      const snapshotChanged = held.identitySnapshot !== bound.identitySnapshot;
      if (snapshotChanged) {
        held.identitySnapshot = bound.identitySnapshot;
        held.identityIndex = bound.identitySnapshot ? IdentityIndex.fromSnapshot(bound.identitySnapshot) : null;
        held.identityProvider = held.identityIndex ? createSnapshotIdentityProvider(held.identityIndex) : undefined;
        held.artProvider = createLaneArtProvider(held.art, { index: held.identityIndex });
      }
      if (held.canvas.width !== widthPx || held.canvas.height !== heightPx) {
        held.canvas.width = widthPx;
        held.canvas.height = heightPx;
        held.picture = createTablePicture();
      }
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const sortSig = `${bound.settings.sort.column}\u0000${bound.settings.sort.direction}`;
      const canReuseModel = held.cachedModel !== null
        && !snapshotChanged
        && held.cachedModelKind === bound.kind
        && samePictures(held.prevParts, input.pictures)
        && held.prevOptions === bound.options
        && held.prevStaticPanels === bound.staticPanels
        && held.prevStaticAnswers === bound.staticAnswers
        && held.prevSortKey === sortSig
        && held.prevExpanded === bound.expanded
        && input.hour >= held.validFrom
        && input.hour < held.validUntil;

      let model: TableModel;
      let flatRows: readonly FlatTableRow[];
      const prevModel = held.cachedModel;
      const prevFlatRows = held.cachedRows;
      const prevRowIndex = held.cachedRowIndex;
      const prevSortKey = held.prevSortKey;
      const prevExpanded = held.prevExpanded;
      const prevValidFrom = held.validFrom;
      const prevValidUntil = held.validUntil;
      const modelChanged = !canReuseModel;
      let incrementalUsed = false;
      if (canReuseModel) {
        model = held.cachedModel!;
        flatRows = held.cachedRows;
      } else {
        const pendingIdentityDemands = new Map<string, readonly [EntityKind, string, number]>();
        const demandedArt = new Set<string>();
        const readView = readViewFromScene(input, bound, input.hour, {
          identity: held.identityProvider,
          onIdentityDemand: (kind, key, atHour) => {
            pendingIdentityDemands.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]);
          },
          art: held.artProvider,
          onArtDemand: handle => {
            const url = artUrlForHandle(handle, held!.identityIndex);
            if (url) demandedArt.add(url);
          },
        });
        model = projection.project(readView, bound.options);
        held.demandedArtUrls = [...demandedArt];
        const demandsList = [...pendingIdentityDemands.values()];
        if (demandsList.length) {
          const dText = JSON.stringify(demandsList);
          held.identityDemandsField = { value: demandsList, content: cachedField(held, 'identityDemands', dText, content) };
        } else {
          held.identityDemandsField = null;
        }
        const modelNextAt = nextTableChange(model, input.hour);
        const canIncResolve = prevModel !== null
          && model.changedKeys !== undefined
          && held.cachedModelKind === bound.kind
          && held.prevOptions === bound.options
          && prevSortKey === sortSig
          && prevExpanded === bound.expanded
          && !held.cachedHasGroupHeaders
          && input.hour >= prevValidFrom
          && input.hour < prevValidUntil;
        const inc = canIncResolve
          ? resolveTableRowsIncremental(prevModel, prevFlatRows, prevRowIndex, model, bound.settings)
          : null;
        let hasGroups: boolean;
        let hasVariableHeights: boolean;
        let rowIndex: Map<string, number>;
        if (inc) {
          flatRows = inc.flatRows;
          rowIndex = inc.rowIndex;
          hasGroups = false;
          hasVariableHeights = false;
          incrementalUsed = true;
        } else {
          flatRows = resolveTableRows(model, bound.settings, bound.expanded);
          hasGroups = false;
          hasVariableHeights = false;
          rowIndex = new Map<string, number>();
          for (let i = 0; i < flatRows.length; i++) {
            const item = flatRows[i]!;
            if (item.kind === 'group-header') {
              hasGroups = true;
              hasVariableHeights = true;
            } else {
              if (item.level > 1) hasVariableHeights = true;
              rowIndex.set(item.key, i);
            }
          }
        }
        let nextBoundary = Math.min(
          nextLookChange(optionsLooks(bound.options), input.hour),
          nextLookChange(bound.theme?.countryLooks, input.hour),
          modelNextAt,
        );
        if (projection.nextChange) {
          nextBoundary = Math.min(nextBoundary, projection.nextChange(readView, bound.options));
        }
        held.prevParts = snapshotPictures(input.pictures);
        held.prevOptions = bound.options;
        held.prevStaticPanels = bound.staticPanels;
        held.prevStaticAnswers = bound.staticAnswers;
        held.prevSortKey = sortSig;
        held.prevExpanded = bound.expanded;
        held.validFrom = input.hour;
        held.validUntil = nextBoundary;
        held.cachedModel = model;
        held.cachedModelKind = bound.kind;
        held.cachedModelNextAt = modelNextAt;
        held.cachedModelHasTimeBoundaries = Number.isFinite(modelNextAt);
        held.cachedRows = flatRows;
        held.cachedHasGroupHeaders = hasGroups;
        held.cachedHasVariableHeights = hasVariableHeights;
        held.cachedRowGeom = null;
        held.cachedRowGeomKey = '';
        held.cachedRowIndex = rowIndex;
      }
      const tProj = timed ? performance.now() : 0;
      if (model.refused !== undefined) {
        held.flusher.cancel();
        return { mode: 'refused', reason: model.refused, surfaces: [], fields: {}, motion: false };
      }

      if (modelChanged) held.sampleSig = textSampleSignature(model.columns, flatRows);
      const rankShown = model.rank === true && (bound.settings.showRank === true || model.defaultRankShown === true);
      const rankMarkersOn = rankShown && model.rankMarkers === true;
      const rankWidth = rankShown ? (bound.rankWidth ?? (rankMarkersOn ? 64 : 36)) : 0;
      const layoutSame = held.cachedLayout !== null
        && sameColumnSpecs(held.layoutCols, model.columns)
        && held.layoutSettings === bound.settings
        && held.layoutWidth === bound.width
        && held.layoutRankShown === rankShown
        && held.layoutRankWidth === rankWidth
        && held.layoutTheme === bound.theme
        && held.layoutSampleSig === held.sampleSig;

      let allLayouts: FamilyColumnLayout[];
      let layout: FamilyTableLayout;
      const layoutChanged = !layoutSame;
      if (layoutSame) {
        allLayouts = held.cachedAllLayouts;
        layout = held.cachedLayout!;
      } else {
        const configKey = `${widthConfigOf(model.columns, bound.settings, bound.theme)}${held.sampleSig}`;
        if (bound.theme && held.widthConfigKey !== configKey) {
          const baseCols = buildColumnLayouts(model.columns, bound.settings);
          const measure = (text: string, font: string) => {
            const cacheKey = `${font}\u0000${text}`;
            let w = held!.picture.measures.get(cacheKey);
            if (w === undefined) {
              held!.ctx.font = font;
              w = held!.ctx.measureText(text).width;
              held!.picture.measures.set(cacheKey, w);
            }
            return w;
          };
          held.fixedWidths = measureFixedColumnWidths(baseCols, bound.settings, bound.theme, measure, initialTextSamples(baseCols, flatRows));
          held.widthConfigKey = configKey;
        }
        allLayouts = buildColumnLayouts(model.columns, bound.settings, held.fixedWidths);
        const protectedKeys = new Set<string>();
        if (!model.unsorted && bound.settings.sort.column) protectedKeys.add(bound.settings.sort.column);
        layout = layoutFamilyTable(allLayouts, Math.max(1, bound.width), bound.settings.overflow, {
          rankShown,
          rankWidth,
          protectedKeys,
        });
        held.layoutCols = model.columns;
        held.layoutSettings = bound.settings;
        held.layoutWidth = bound.width;
        held.layoutRankShown = rankShown;
        held.layoutRankWidth = rankWidth;
        held.layoutTheme = bound.theme;
        held.layoutSampleSig = held.sampleSig;
        held.cachedAllLayouts = allLayouts;
        held.cachedLayout = layout;
        held.cachedMaxima = null;
        held.cachedMaximaCols = '';
        held.rowAuditCache = new WeakMap();
        const widthsPayload = {
          widths: Object.fromEntries([...allLayouts.filter(col => col.shown), ...layout.columns].map(col => [col.key, col.width])),
          visible: layout.columns.map(col => col.key),
          hidden: layout.hidden,
          totalWidth: layout.totalWidth,
          scrollable: layout.scrollable,
          pinnedCount: layout.pinnedCount,
          moreWidth: layout.moreWidth,
        };
        const widthsText = JSON.stringify(widthsPayload);
        held.widthsField = { value: widthsPayload, content: cachedField(held, 'widths', widthsText, content) };
      }

      const now = laneOptions?.clock ? laneOptions.clock() : input.now;
      const glideMs = glideDurationForSpeed(input.speed, bound.reducedMotion === true);
      const rowHeightMode = model.rowHeightMode ?? 'standard';
      const rowHeight = rowHeightMode === 'media' ? MEDIA_ROW_HEIGHT[bound.settings.density] : ROW_HEIGHT[bound.settings.density];
      let rowGeom: RowLayoutGeometry | null = null;
      if (held.cachedHasVariableHeights) {
        const geomKey = `${bound.settings.density}:${rowHeightMode}`;
        if (held.cachedRowGeom && held.cachedRowGeomKey === geomKey) {
          rowGeom = held.cachedRowGeom;
        } else {
          rowGeom = computeRowLayout(flatRows, bound.settings.density, rowHeightMode);
          held.cachedRowGeom = rowGeom;
          held.cachedRowGeomKey = geomKey;
        }
      }
      const totalHeight = rowGeom ? rowGeom.totalHeight : flatRows.length * rowHeight;
      if (modelChanged) {
        const forwardStep = input.speed > 0 && Number.isFinite(held.priorHour) && model.hour > held.priorHour;
        const jumped = !Number.isFinite(held.priorHour) || model.hour < held.priorHour || (model.hour !== held.priorHour && !forwardStep);
        if (held.priorRows.length && glideMs > 0 && !jumped) {
          const oldIndex = new Map<string, number>();
          held.priorRows.forEach((item, index) => { if (item.kind === 'data') oldIndex.set(item.key, index); });
          const nextGlide = new Map<string, GlideOffset>();
          flatRows.forEach((item, index) => {
            if (item.kind !== 'data') return;
            const before = oldIndex.get(item.key);
            if (before !== undefined && before !== index) {
              nextGlide.set(item.key, { from: (before - index) * rowHeight, to: 0, started: now });
            } else {
              const existing = held!.glide.get(item.key);
              if (existing && now - existing.started < glideMs) nextGlide.set(item.key, existing);
            }
          });
          held.glide = nextGlide;
        } else {
          held.glide = new Map();
        }

        if (rankMarkersOn) {
          const dataKeys = flatRows.filter(item => item.kind === 'data').map(item => item.key);
          const holdMs = rankHoldForSpeed(input.speed);
          held.markers = forwardStep && held.order?.sort === sortSig ? rankMarkers(held.order.keys, dataKeys, now, held.markers, holdMs) : new Map();
          held.order = { sort: sortSig, keys: dataKeys };
        } else {
          held.markers = new Map();
        }
        held.priorRows = flatRows;
        held.priorHour = model.hour;
      } else {
        if (held.glide.size) {
          for (const [key, g] of held.glide) if (glideMs === 0 || now - g.started >= glideMs) held.glide.delete(key);
        }
        if (held.markers.size) {
          for (const [key, m] of held.markers) if (m.until <= now) held.markers.delete(key);
        }
      }
      const tLayout = timed ? performance.now() : 0;

      const inspectedKey = typeof input.inspect === 'string' && input.inspect.startsWith('row:') ? input.inspect.slice(4) : null;
      const selectedKey = bound.selectedKey ?? null;
      const focusedKey = bound.focusedKey ?? null;

      let maximaCols = '';
      for (let c = 0; c < layout.columns.length; c++) {
        const col = layout.columns[c]!;
        if (col.numeric) maximaCols += `${col.key}:${col.display}|`;
      }
      let maxima: Map<string, number>;
      if (!modelChanged && held.cachedMaxima && held.cachedMaximaCols === maximaCols) {
        maxima = held.cachedMaxima;
      } else if (
        incrementalUsed
        && held.cachedMaxima
        && held.cachedMaximaCols === maximaCols
        && model.changedKeys
      ) {
        maxima = model.changedKeys.size === 0
          ? held.cachedMaxima
          : updateColumnMaximaIncremental(
              held.cachedMaxima,
              prevFlatRows,
              prevRowIndex,
              flatRows,
              held.cachedRowIndex,
              layout.columns,
              model.changedKeys,
              prevModel!.hour,
              model.hour,
            );
        held.cachedMaxima = maxima;
      } else {
        maxima = computeColumnMaxima(flatRows, layout.columns, model.hour);
        held.cachedMaxima = maxima;
        held.cachedMaximaCols = maximaCols;
      }

      const visibleFirst = Math.max(0, Math.floor(bound.scrollTop / rowHeight));
      const visibleLast = Math.min(flatRows.length, Math.ceil((bound.scrollTop + bound.height) / rowHeight) + (held.glide.size && glideMs > 0 ? 2 : 0));
      const visibleIcons = collectVisibleCellIcons(flatRows, layout.columns, model.hour, visibleFirst, visibleLast);
      held.art.requestVisible(held.demandedArtUrls.length ? [...visibleIcons, ...held.demandedArtUrls] : visibleIcons);

      let painted = false;
      if (bound.theme) {
        const result = drawFamilyTable(held.ctx, {
          layout,
          rows: flatRows,
          settings: bound.settings,
          hour: model.hour,
          width: bound.width,
          height: bound.height,
          scrollLeft: bound.scrollLeft,
          scrollTop: bound.scrollTop,
          rankShown,
          rankWidth,
          theme: bound.theme,
          now,
          glideDuration: glideMs,
          glide: held.glide,
          markers: held.markers,
          inspectedKey,
          selectedKey,
          focusedKey,
          empty: model.empty,
          icons: held.art.map,
          iconVersion: held.art.version,
          artCache: held.art,
          dpr,
          hasGroupHeaders: held.cachedHasGroupHeaders,
          precomputedMaxima: maxima,
          staticRowLooks: model.rowTimeBoundaries === false,
          rowIndex: held.cachedRowIndex,
          rowHeightMode,
          rowGeom,
          clockSummary: model.clockSummary,
        }, held.picture);
        painted = result.drew;
      }
      if (painted) held.content = content();
      const tPaint = timed ? performance.now() : 0;

      const moving = glideMs > 0 && held.glide.size > 0;
      const setTimer = laneOptions?.cadenceTimers?.set ?? setTimeout;
      const clearTimer = laneOptions?.cadenceTimers?.clear ?? clearTimeout;
      if (held.markers.size > 0) {
        let earliest = Infinity;
        for (const m of held.markers.values()) if (m.until < earliest) earliest = m.until;
        if (Number.isFinite(earliest) && (held.markerTimer === null || earliest < held.markerTimerAt)) {
          if (held.markerTimer !== null) clearTimer(held.markerTimer);
          held.markerTimerAt = earliest;
          const holder = held;
          const delay = Math.max(1, Math.ceil(earliest - now));
          held.markerTimer = setTimer(() => {
            holder.markerTimer = null;
            holder.markerTimerAt = Infinity;
            holder.wake?.();
          }, delay);
          (held.markerTimer as unknown as { unref?: () => void })?.unref?.();
        }
      } else if (held.markerTimer !== null) {
        clearTimer(held.markerTimer);
        held.markerTimer = null;
        held.markerTimerAt = Infinity;
      }
      const fields: Record<string, FieldEntry> = {};

      if (held.widthsField) fields.widths = held.widthsField;
      if (held.identityDemandsField) fields.identityDemands = held.identityDemandsField;

      const hasTree = !incrementalUsed && flatRows.some(item => item.kind === 'data' && item.isTreeGroup);
      if (modelChanged || layoutChanged || !held.metaField) {
        const mSig = metaSignature(model, hasTree, rankShown, rankWidth);
        if (mSig !== held.metaSig || !held.metaField) {
          held.metaSig = mSig;
          held.metaField = {
            value: {
              empty: model.empty ?? null,
              emptyDetail: model.emptyDetail ?? null,
              emptyKind: model.emptyKind ?? 'empty',
              titleSummary: model.titleSummary ?? null,
              titleSummaryPartial: model.titleSummaryPartial ?? null,
              hideHeader: model.hideHeader === true,
              rowHeightMode,
              notes: model.notes ?? [],
              warnings: model.warnings ?? [],
              hasTree,
              unsorted: model.unsorted === true,
              rankShown,
              rankWidth,
              columns: model.columns,
            },
            content: content(),
          };
        }
      }
      fields.meta = held.metaField!;

      const navUrgent = !held.navField || input.speed === 0 || focusedKey !== null || hasTree || (modelChanged && !incrementalUsed) || (held.navItems !== null && held.navItems.length !== flatRows.length && !held.cachedHasGroupHeaders);
      if (modelChanged) held.navGate.invalidate();
      if (navUrgent || held.navGate.due(now, input.speed, `${flatRows.length}:${focusedKey ?? ''}`)) {
        if (!held.navField || !sameNavItems(held.navItems, flatRows)) {
          const navItems: NavTuple[] = [];
          for (let i = 0; i < flatRows.length; i++) {
            const item = flatRows[i]!;
            if (item.kind === 'data') {
              navItems.push([item.key, item.name, i, item.isTreeGroup ? (item.isOpen ? 1 : 0) : -1, item.parent ?? '', item.row.meta ?? null]);
            }
          }
          held.navItems = navItems;
          held.navField = { value: navItems, content: content() };
        }
        held.navGate.record(now, `${flatRows.length}:${focusedKey ?? ''}`);
      }
      fields.nav = held.navField!;

      const focusedIndex = focusedKey ? (held.cachedRowIndex.get(focusedKey) ?? -1) : -1;
      const inspectedPresent = inspectedKey ? held.cachedRowIndex.has(inspectedKey) : false;
      const tSig = `${flatRows.length}:${totalHeight}:${focusedIndex}:${inspectedPresent ? 1 : 0}`;
      if (tSig !== held.tableSig || !held.tableField) {
        held.tableSig = tSig;
        held.tableField = { value: { count: flatRows.length, totalHeight, focusedIndex, inspectedPresent }, content: content() };
      }
      fields.table = held.tableField;

      const mirrorKey = `${bound.window ? `${bound.window.first}:${bound.window.count}` : ''}\u0000${focusedKey ?? ''}`;
      if (modelChanged || layoutChanged) held.mirrorGate.invalidate();
      if (!held.mirrorField || held.mirrorGate.due(now, input.speed, mirrorKey)) {
        const visibleSpecs = layout.columns.map(col => model.columns.find(spec => spec.key === col.key)!).filter(Boolean);
        const mirror = deriveTableMirror(flatRows, visibleSpecs, model.hour, rankShown, bound.window, focusedKey, focusedIndex);
        const mirrorText = JSON.stringify(mirror);
        held.mirrorField = { value: mirror, content: cachedField(held, 'mirror', mirrorText, content) };
        held.mirrorGate.record(now, mirrorKey);
      }
      if (held.mirrorField) fields.mirror = held.mirrorField;

      const hiddenSet = (inspectedKey || focusedKey) ? new Set(layout.hidden) : null;
      let builtTipKey: string | null = null;
      let builtTip: ReturnType<typeof deriveTableTooltip> = null;
      const getTip = (rowKey: string) => {
        if (builtTipKey === rowKey) return builtTip;
        builtTipKey = rowKey;
        builtTip = deriveTableTooltip(flatRows, rowKey, model.columns, hiddenSet!, model.hour);
        return builtTip;
      };

      const stringifyTip = (payload: unknown): string => JSON.stringify(payload, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v));

      if (inspectedKey) {
        if (modelChanged || layoutChanged) held.tipGate.invalidate();
        if (!held.tipField || held.tipGate.due(now, input.speed, inspectedKey)) {
          const tooltip = getTip(inspectedKey);
          const tipPayload = { key: `row:${inspectedKey}`, text: tooltip ? { hour: model.hour, rowKey: inspectedKey, content: tooltip } : null };
          const tipText = stringifyTip(tipPayload);
          held.tipField = { value: tipPayload, content: cachedField(held, 'tooltip', tipText, content) };
          held.tipGate.record(now, inspectedKey);
        }
        if (held.tipField) fields.tooltip = held.tipField;
      } else {
        held.tipGate.reset();
        held.tipField = null;
      }

      if (focusedKey) {
        if (modelChanged || layoutChanged) held.focusTipGate.invalidate();
        if (!held.focusTipField || held.focusTipGate.due(now, input.speed, focusedKey)) {
          const tooltip = getTip(focusedKey);
          const tipPayload = { key: `focus:${focusedKey}`, text: tooltip ? { hour: model.hour, rowKey: focusedKey, content: tooltip } : null };
          const tipText = stringifyTip(tipPayload);
          held.focusTipField = { value: tipPayload, content: cachedField(held, 'focusTooltip', tipText, content) };
          held.focusTipGate.record(now, focusedKey);
        }
        if (held.focusTipField) fields.focusTooltip = held.focusTipField;
      } else {
        held.focusTipGate.reset();
        held.focusTipField = null;
      }

      const target = held;
      held.flusher.sync(now, input.speed, () => { target.wake?.(); });
      const tFields = timed ? performance.now() : 0;

      if (bound.audit) {
        const nextAuditId = bound.auditId ?? -1;
        const winKey = bound.window ? `${bound.window.first}:${bound.window.count}` : '';
        if (modelChanged || layoutChanged || !held.auditField || held.auditId !== nextAuditId || held.auditWindowKey !== winKey) {
          const auditRows = deriveTableAudit({
            model,
            flatRows,
            visibleColumns: layout.columns,
            hiddenKeys: new Set(layout.hidden),
            rankShown,
            maxima,
            window: bound.window,
            countryColors: bound.theme,
            rowAuditCache: held.rowAuditCache,
          });
          held.auditId = nextAuditId;
          held.auditWindowKey = winKey;
          held.auditField = { value: { id: nextAuditId, rows: auditRows }, content: content() };
        }
        fields.audit = held.auditField;
      } else {
        held.auditField = null;
        held.auditId = -1;
      }

      if (timed) {
        const tEnd = performance.now();
        held.phases.serves++;
        held.phases.projMs += tProj - t0;
        held.phases.layoutMs += tLayout - tProj;
        held.phases.paintMs += tPaint - tLayout;
        held.phases.fieldsMs += tFields - tPaint;
        held.phases.auditMs += tEnd - tFields;
        if (input.speed === 0 || held.phases.serves % 30 === 0 || !held.phasesField) {
          held.phasesField = { value: { kind: bound.kind, ...held.phases }, content: content() };
        }
        fields.phases = held.phasesField;
      }

      const glideSnapshot = moving ? new Map(held.glide) : undefined;
      const hit = (point: readonly [number, number]) => {
        if (model.clockSummary) return null;
        const at = hitFamilyTable(layout, flatRows, { x: point[0], y: point[1] }, rowHeight, bound.scrollLeft, bound.scrollTop, rankShown, rankWidth, glideSnapshot, now, glideMs, rowGeom, held!.cachedRowIndex, bound.height);
        if (at.kind !== 'row' || !at.rowKey) return null;
        return at.toggle ? `toggle:${at.rowKey}` : `row:${at.rowKey}`;
      };

      const handledPicture = model.rows.length > 0 || model.empty !== undefined || model.clockSummary !== undefined;
      return {
        ...(handledPicture ? { mode: 'picture' as const } : {}),
        ...(Number.isFinite(held.cachedModelNextAt) ? { nextChangeAt: held.cachedModelNextAt } : {}),
        viewport: {
          width: bound.width,
          height: bound.height,
          dpr: input.binding.surfaces[0]?.dpr ?? 1,
          scrollLeft: bound.scrollLeft,
          scrollTop: bound.scrollTop,
        },
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: moving,
        hit,
      };
    },
  };
}
