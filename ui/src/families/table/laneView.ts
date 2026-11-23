import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import { ROW_HEIGHT, type GlideOffset, type TableSettings } from '../../draw/tables/logic.ts';
import { rankMarkers, type RankMarker, type TableTheme } from '../../draw/tables/surface.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import type { LiveValues } from '../../charts/live';
import type { FamilySources } from '../contract.ts';
import { buildColumnLayouts, layoutFamilyTable, measureFixedColumnWidths, type FamilyColumnLayout, type FamilyTableLayout } from './layout.ts';
import {
  deriveTableAudit,
  deriveTableMirror,
  deriveTableTooltip,
  nextTableChange,
  resolveTableRows,
  type FlatTableRow,
  type TableColumnSpec,
  type TableModel,
} from './model.ts';
import {
  collectCellIcons,
  createTablePicture,
  drawFamilyTable,
  glideDurationForSpeed,
  hitFamilyTable,
  rankHoldForSpeed,
  type FamilyTablePicture,
} from './painter.ts';
import { tableProjection } from './registry.ts';

export interface FamilyTableBinding<TOptions = unknown> {
  readonly family: 'table';
  readonly kind: string;
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
  readonly rankWidth?: number;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly expanded?: Readonly<Record<string, boolean>>;
  readonly reducedMotion?: boolean;
  readonly window?: { readonly first: number; readonly count: number };
  readonly audit?: boolean;
}

export function isFamilyTableBinding(value: unknown): value is FamilyTableBinding {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'table' && typeof (value as { kind?: unknown }).kind === 'string';
}

interface HeldFamilyTable {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  picture: FamilyTablePicture;
  content: string;
  icons: Map<string, ImageBitmap | null>;
  widthConfigKey: string;
  fixedWidths: Record<string, number>;
  priorRows: readonly FlatTableRow[];
  priorHour: number;
  glide: Map<string, GlideOffset>;
  order?: { sort: string; keys: string[] };
  markers: Map<string, RankMarker>;
  kept: Map<string, { text: string; content: string }>;
  wake?: () => void;
}

const liveAdapter = (map: LiveMap | null): LiveValues<number | bigint | string> | null => map && {
  get: (key: string) => {
    const value = map.get(key);
    return typeof value === 'number' || typeof value === 'bigint' ? value : undefined;
  },
  raw: (key: string) => {
    const value = map.get(key);
    return typeof value === 'bigint' ? value : typeof value === 'number' && Number.isSafeInteger(value) ? BigInt(value) : undefined;
  },
  missing: (key: string) => {
    const value = map.get(key);
    return typeof value === 'string' ? value : 'Not recorded';
  },
};

function answerOf(value: LaneValue): FollowedAnswer | undefined {
  return value.kind === 'answer' ? value.answer : undefined;
}

export function sourcesFromScene(scene: Scene, binding: FamilyTableBinding): FamilySources {
  const answers = new Map<string, FollowedAnswer | undefined>();
  const panels = new Map<string, unknown>();
  const live = new Map<string, LiveValues<number | bigint | string> | null>();
  if (binding.staticAnswers) {
    for (const [name, answer] of Object.entries(binding.staticAnswers)) answers.set(name, answer);
  }
  if (binding.staticPanels) {
    for (const [name, data] of Object.entries(binding.staticPanels)) {
      panels.set(name, data);
      live.set(name, repliedCells(data).read());
    }
  }
  for (const [name, subId] of Object.entries(binding.sources)) {
    const at = scene.subs.findIndex(sub => sub.id === subId);
    const picture = at < 0 ? undefined : scene.pictures[at];
    const head = picture?.parts[0]?.value;
    if (!head) continue;
    if (head.kind === 'answer') {
      answers.set(name, answerOf(head));
    } else if (head.kind === 'panel' && head.refused === null) {
      panels.set(name, head.data);
      live.set(name, liveAdapter(liveValues(picture!)));
    }
  }
  return { answers, panels, live };
}

function loadIcon(held: HeldFamilyTable, url: string) {
  if (held.icons.has(url)) return;
  held.icons.set(url, null);
  void fetch(url)
    .then(response => { if (!response.ok) throw new Error(`icon ${response.status}`); return response.blob(); })
    .then(blob => createImageBitmap(blob))
    .then(image => { held.icons.set(url, image); held.picture = createTablePicture(); held.wake?.(); }, () => {});
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
    columns.map(c => [c.key, c.label, c.as, c.widest ?? null, c.format ?? null, c.numeric ?? false]),
    settings.order,
    Object.fromEntries(Object.entries(settings.columns).map(([k, v]) => [k, [v.show, v.as, v.width]])),
    theme?.font ?? '',
    theme?.fontSize ?? null,
  ]);
}

function initialTextSamples(columns: readonly FamilyColumnLayout[], rows: readonly FlatTableRow[], hour: number): Record<string, string[]> {
  const samples: Record<string, string[]> = {};
  const limit = Math.min(rows.length, 40);
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
  void hour;
  return samples;
}

export function familyTableLaneView(wake: (widget: number) => void): LaneView {
  return {
    next(scene: Scene, hour: number): number {
      const bound = scene.model;
      if (!isFamilyTableBinding(bound)) return Infinity;
      const projection = tableProjection(bound.kind);
      if (!projection) return Infinity;
      const sources = sourcesFromScene(scene, bound);
      let soonest = projection.nextChange ? projection.nextChange(sources, hour, bound.options) : Infinity;
      const model = projection.project(sources, hour, bound.options);
      soonest = Math.min(soonest, nextTableChange(model, hour));
      return soonest;
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyTableBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };
      const projection = tableProjection(bound.kind);
      if (!projection) {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown table declaration: ${bound.kind}` };
      }

      let held = state.get('familyTable') as HeldFamilyTable | undefined;
      const dpr = surface.dpr || 1;
      const widthPx = Math.max(1, Math.round(bound.width * dpr));
      const heightPx = Math.max(1, Math.round(bound.height * dpr));
      if (!held) {
        const canvas = new OffscreenCanvas(widthPx, heightPx);
        const ctx = canvas.getContext('2d')!;
        state.set('familyTable', held = {
          canvas,
          ctx,
          picture: createTablePicture(),
          content: content(),
          icons: new Map(),
          widthConfigKey: '',
          fixedWidths: {},
          priorRows: [],
          priorHour: -Infinity,
          glide: new Map(),
          markers: new Map(),
          kept: new Map(),
        });
      }
      held.wake = () => wake(input.widget);
      if (held.canvas.width !== widthPx || held.canvas.height !== heightPx) {
        held.canvas.width = widthPx;
        held.canvas.height = heightPx;
        held.picture = createTablePicture();
      }
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const sources = sourcesFromScene(input, bound);
      const model: TableModel = projection.project(sources, input.hour, bound.options);
      const flatRows = resolveTableRows(model, bound.settings, bound.expanded);
      for (const url of collectCellIcons(flatRows)) loadIcon(held, url);

      const configKey = widthConfigOf(model.columns, bound.settings, bound.theme);
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
        held.fixedWidths = measureFixedColumnWidths(baseCols, bound.settings, bound.theme, measure, initialTextSamples(baseCols, flatRows, model.hour));
        held.widthConfigKey = configKey;
      }

      const rankShown = model.rank === true;
      const rankMarkersOn = rankShown && model.rankMarkers === true;
      const rankWidth = rankShown ? (bound.rankWidth ?? (rankMarkersOn ? 64 : 36)) : 0;
      const allLayouts = buildColumnLayouts(model.columns, bound.settings, held.fixedWidths);
      const protectedKeys = new Set<string>();
      if (!model.unsorted && bound.settings.sort.column) protectedKeys.add(bound.settings.sort.column);
      const layout: FamilyTableLayout = layoutFamilyTable(allLayouts, Math.max(1, bound.width), bound.settings.overflow, {
        rankShown,
        rankWidth,
        protectedKeys,
      });

      const now = input.now;
      const glideMs = glideDurationForSpeed(input.speed, bound.reducedMotion === true);
      const rowHeight = ROW_HEIGHT[bound.settings.density];
      if (held.priorRows.length && glideMs > 0) {
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
        const sortSig = `${bound.settings.sort.column}\u0000${bound.settings.sort.direction}`;
        const holdMs = rankHoldForSpeed(input.speed);
        held.markers = held.order?.sort === sortSig ? rankMarkers(held.order.keys, dataKeys, now, held.markers, holdMs) : new Map();
        held.order = { sort: sortSig, keys: dataKeys };
      } else {
        held.markers = new Map();
      }

      held.priorRows = flatRows;
      held.priorHour = model.hour;

      const inspectedKey = typeof input.inspect === 'string' && input.inspect.startsWith('row:') ? input.inspect.slice(4) : null;
      const selectedKey = bound.selectedKey ?? null;
      const focusedKey = bound.focusedKey ?? null;

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
          icons: held.icons,
          dpr,
        }, held.picture);
        painted = result.drew;
      }
      if (painted) held.content = content();

      const moving = (glideMs > 0 && held.glide.size > 0) || [...held.markers.values()].some(marker => marker.until > now);
      const fields: Record<string, { value: unknown; content: string }> = {};

      const widthsPayload = {
        widths: Object.fromEntries(layout.columns.map(col => [col.key, col.width])),
        visible: layout.columns.map(col => col.key),
        hidden: layout.hidden,
        totalWidth: layout.totalWidth,
        scrollable: layout.scrollable,
        moreWidth: layout.moreWidth,
      };
      const widthsText = JSON.stringify(widthsPayload);
      fields.widths = { value: widthsPayload, content: cachedField(held, 'widths', widthsText, content) };

      const hasTree = flatRows.some(item => item.kind === 'data' && item.isTreeGroup);
      const metaPayload = {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        summaryPill: model.summaryPill ?? null,
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
        hasTree,
        unsorted: model.unsorted === true,
        rankShown,
        rankWidth,
        columns: model.columns,
      };
      const metaText = JSON.stringify(metaPayload);
      fields.meta = { value: metaPayload, content: cachedField(held, 'meta', metaText, content) };

      const focusedIndex = focusedKey ? flatRows.findIndex(item => item.kind === 'data' && item.key === focusedKey) : -1;
      const inspectedPresent = inspectedKey ? flatRows.some(item => item.kind === 'data' && item.key === inspectedKey) : false;
      const navItems = flatRows.flatMap((item, index) => item.kind === 'data'
        ? [[item.key, item.name, index, item.isTreeGroup ? (item.isOpen ? 1 : 0) : -1, item.parent ?? '', item.row.meta ?? null] as const]
        : []);
      const tablePayload = { count: flatRows.length, focusedIndex, inspectedPresent, nav: navItems };
      const tableText = JSON.stringify(tablePayload);
      fields.table = { value: tablePayload, content: cachedField(held, 'table', tableText, content) };

      const visibleSpecs = layout.columns.map(col => model.columns.find(spec => spec.key === col.key)!).filter(Boolean);
      const mirror = deriveTableMirror(flatRows, visibleSpecs, model.hour, rankShown, bound.window, focusedKey);
      const mirrorText = JSON.stringify(mirror);
      fields.mirror = { value: mirror, content: cachedField(held, 'mirror', mirrorText, content) };

      const tipKey = inspectedKey ?? focusedKey;
      if (tipKey) {
        const hiddenSet = new Set(layout.hidden);
        const tooltip = deriveTableTooltip(flatRows, tipKey, model.columns, hiddenSet, model.hour);
        const tipPayload = { key: inspectedKey ? `row:${inspectedKey}` : `focus:${focusedKey}`, text: tooltip ? { hour: model.hour, rowKey: tipKey, content: tooltip } : null };
        const tipText = JSON.stringify(tipPayload);
        fields.tooltip = { value: tipPayload, content: cachedField(held, 'tooltip', tipText, content) };
      }

      if (bound.audit) {
        const enabledSpecs = allLayouts.filter(col => col.shown).map(col => model.columns.find(spec => spec.key === col.key)!).filter(Boolean);
        const auditRows = deriveTableAudit(flatRows, enabledSpecs, model.hour, model.empty);
        const auditText = JSON.stringify(auditRows);
        fields.audit = { value: auditRows, content: cachedField(held, 'audit', auditText, content) };
      }

      const hit = (point: readonly [number, number]) => {
        const at = hitFamilyTable(layout, flatRows, { x: point[0], y: point[1] }, rowHeight, bound.scrollLeft, bound.scrollTop, rankShown, rankWidth);
        if (at.kind !== 'row' || !at.rowKey) return null;
        return at.toggle ? `toggle:${at.rowKey}` : `row:${at.rowKey}`;
      };

      return { surfaces: [{ source: held.canvas, content: held.content }], fields, motion: moving, hit };
    },
  };
}
