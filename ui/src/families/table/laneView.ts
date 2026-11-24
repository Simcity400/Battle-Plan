import { canvasMeasureText } from '../../charts/textWidth.ts';
import { collectArtUrls, type ArtUrlInput } from '../art/provider.ts';
import type { FamilyAuditRow } from '../auditRow.ts';
import type { Surface } from '../../draw/lane.ts';
import type { FamilyTheme } from '../contract.ts';
import type { LaneBinding, LaneFamily, LaneFrame, LaneStep } from '../laneRuntime.ts';
import { buildColumnLayouts, computeRowLayout, layoutFamilyTable, measureFixedColumnWidths, tableRowWindow, type FamilyColumnLayout, type FamilyTableLayout, type RowLayoutGeometry } from './layout.ts';
import {
  computeColumnMaxima,
  deriveTableAudit,
  deriveTableMirror,
  deriveTableTooltip,
  keepsProductOrder,
  nextTableChange,
  resolveCell,
  resolveTableRows,
  resolveTableRowsIncremental,
  tableRank,
  updateColumnMaximaIncremental,
  type FlatTableRow,
  type TableColumnSpec,
  type TableModel,
  type TableRow,
} from './model.ts';
import { createTablePicture, drawFamilyTable, drawPaneFrame, glideDurationForSpeed, hitFamilyTable, hitPanes, rankHoldForSpeed, type FamilyTablePicture } from './painter.ts';
import { combinedPaneGeometry, combinedPaneRows, paneGeometry, paneOfRow, panesHeight, paneScroll, paneTitleText, resolvePanes, visiblePaneRange, type ResolvedPane } from './panes.ts';
import { MEDIA_ROW_HEIGHT, ROW_HEIGHT, rankMarkers, showsIcon, type GlideOffset, type RankMarker, type TableSettings } from './settings.ts';

export interface FamilyTableBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'table';
  readonly settings: TableSettings;
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly expanded?: Readonly<Record<string, boolean>>;
  readonly reducedMotion?: boolean;
  readonly pauseFlush?: number;
}

const TEXT_SAMPLE_ROWS = 40;
const shapes = new WeakMap<TableModel, TableShape>();

interface TableShape {
  readonly model: TableModel;
  readonly options: unknown;
  readonly settings: TableSettings;
  readonly expanded: unknown;
  readonly width: number;
  readonly theme: FamilyTheme;
  readonly flatRows: readonly FlatTableRow[];
  readonly rowIndex: Map<string, number>;
  readonly hasGroups: boolean;
  readonly incremental: boolean;
  readonly hasTree: boolean;
  readonly panes: readonly ResolvedPane[] | null;
  readonly paneGeom: ReturnType<typeof paneGeometry> | null;
  readonly rankShown: boolean;
  readonly rankMarkersOn: boolean;
  readonly rankWidth: number;
  readonly widthKey: string;
  readonly fixedWidths: Record<string, number>;
  readonly allLayouts: FamilyColumnLayout[];
  readonly columns: FamilyTableLayout;
  readonly rowHeightMode: 'standard' | 'media';
  readonly rowHeight: number;
  readonly rowGeom: RowLayoutGeometry | null;
  readonly windowOffset: number;
  readonly totalCount: number;
  readonly totalHeight: number;
  readonly maxima: Map<string, number>;
  readonly maximaCols: string;
  readonly rowAuditCache: WeakMap<TableRow, { readonly sig: string; readonly rows: readonly FamilyAuditRow[] }>;
  readonly sortSig: string;
  readonly orderKeys: readonly string[];
  readonly markers: ReadonlyMap<string, RankMarker>;
  readonly glide: ReadonlyMap<string, GlideOffset>;
}

interface TableLayout {
  readonly shape: TableShape;
  readonly localScrollTop: number;
  readonly rowWindow: { readonly first: number; readonly count: number };
  readonly tops: (index: number) => number;
  readonly heights: (index: number) => number;
  readonly focusedIndex: number;
  readonly height: number;
}

type TableFrame = LaneFrame<FamilyTableBinding, TableModel, TableLayout>;

const pictures = new WeakMap<OffscreenCanvasRenderingContext2D, { width: number; height: number; art: number; list: FamilyTablePicture[] }>();

function picturesFor(ctx: OffscreenCanvasRenderingContext2D, surface: Surface, art: number): FamilyTablePicture[] {
  const width = surface.width * surface.dpr;
  const height = surface.height * surface.dpr;
  let held = pictures.get(ctx);
  if (!held || held.width !== width || held.height !== height || held.art !== art) pictures.set(ctx, held = { width, height, art, list: [] });
  return held.list;
}

function widthConfigOf(columns: readonly TableColumnSpec[], settings: TableSettings, theme: FamilyTheme, hideHeader: boolean): string {
  return JSON.stringify([
    hideHeader,
    settings.sort.column,
    theme.font,
    theme.fontSize.small,
    theme.fontSize.body,
    settings.order,
    columns.map(c => {
      const own = settings.columns[c.key];
      return [c.key, c.label, c.as, own?.show ?? true, own?.as ?? '', own?.width ?? null, c.numeric ? 1 : 0, c.cellKind ?? ''];
    }),
  ]);
}

function textSamples(columns: readonly { readonly key: string; readonly numeric?: boolean; readonly widest?: readonly string[] }[], rows: readonly FlatTableRow[]): Record<string, string[]> {
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
      const val = cell.value.kind === 'known' ? String(cell.value.value) : '';
      if (val) list.push(val);
    }
    samples[column.key] = list;
  }
  return samples;
}

function sampleSignature(samples: Record<string, string[]>): string {
  let sig = '';
  for (const [key, list] of Object.entries(samples)) {
    let longest = 0;
    for (const text of list) if (text.length > longest) longest = text.length;
    sig += `${key}:${longest}|`;
  }
  return sig;
}

function shapeOf(model: TableModel, binding: FamilyTableBinding, theme: FamilyTheme, width: number, measure: OffscreenCanvasRenderingContext2D, step: LaneStep<TableModel, TableLayout>): TableShape {
  const prev = step.previous?.layout.shape ?? null;
  const settings = binding.settings;
  const expanded = binding.expanded;
  const held = shapes.get(model);
  if (held && held.options === binding.options && held.settings === settings && held.expanded === expanded && held.width === width && held.theme === theme) return held;
  const sortSig = `${settings.sort.column}\u0000${settings.sort.direction}`;
  const sameView = prev !== null && prev.options === binding.options && prev.settings === settings && prev.expanded === expanded;
  const inc = sameView && prev.model !== model && model.changedKeys !== undefined && !prev.hasGroups && !prev.panes && model.panes === undefined
    ? resolveTableRowsIncremental(prev.model, prev.flatRows, prev.rowIndex, model, settings)
    : null;
  const rowHeightMode = model.rowHeightMode ?? 'standard';
  let flatRows: readonly FlatTableRow[];
  let rowIndex: Map<string, number>;
  let hasGroups = false;
  let hasVariableHeights = false;
  let panes: readonly ResolvedPane[] | null = null;
  if (inc) {
    flatRows = inc.flatRows;
    rowIndex = inc.rowIndex;
  } else {
    panes = model.panes ? resolvePanes(model, settings, expanded, rowHeightMode) : null;
    flatRows = panes ? combinedPaneRows(panes) : resolveTableRows(model, settings, expanded);
    rowIndex = new Map();
    for (let i = 0; i < flatRows.length; i++) {
      const item = flatRows[i]!;
      if (item.kind === 'group-header') {
        hasGroups = true;
        hasVariableHeights = true;
      } else if (item.kind !== 'data') {
        hasVariableHeights = true;
        if (item.kind === 'gap' || item.kind === 'refused') rowIndex.set(item.key, i);
      } else {
        if (item.level > 1 || item.isLiveEnd === true) hasVariableHeights = true;
        rowIndex.set(item.key, i);
      }
    }
  }
  const { shown: rankShown, markers: rankMarkersOn, width: rankWidth } = tableRank(model, settings);
  const paneGeom = panes ? paneGeometry(width, panes.length) : null;
  const layoutWidth = paneGeom ? paneGeom.width : width;
  const baseCols = buildColumnLayouts(model.columns, settings);
  const samples = textSamples(baseCols, flatRows);
  const widthKey = `${widthConfigOf(model.columns, settings, theme, model.hideHeader === true)}${sampleSignature(samples)}`;
  const fixedWidths = prev && prev.widthKey === widthKey
    ? prev.fixedWidths
    : measureFixedColumnWidths(baseCols, settings, theme, canvasMeasureText(measure), samples, { hideHeader: model.hideHeader === true });
  const allLayouts = buildColumnLayouts(model.columns, settings, fixedWidths);
  const columns = layoutFamilyTable(allLayouts, Math.max(1, layoutWidth), { rankShown, rankWidth });
  const rowHeight = rowHeightMode === 'media' ? MEDIA_ROW_HEIGHT[settings.density] : ROW_HEIGHT[settings.density];
  const rowGeom = hasVariableHeights ? computeRowLayout(flatRows, settings.density, rowHeightMode) : null;
  const localTotal = panes ? panesHeight(panes) : rowGeom ? rowGeom.totalHeight : flatRows.length * rowHeight;
  const windowOffset = panes ? 0 : (model.windowOffset ?? 0);
  const totalCount = panes ? flatRows.length : Math.max(windowOffset + flatRows.length, model.totalRows ?? flatRows.length);
  const totalHeight = localTotal + (panes ? 0 : Math.max(0, totalCount - flatRows.length) * rowHeight);
  let maximaCols = '';
  for (const col of columns.columns) if (col.numeric) maximaCols += `${col.key}:${col.display}|`;
  const maxima = inc && prev && prev.maximaCols === maximaCols && model.changedKeys
    ? (model.changedKeys.size === 0
      ? prev.maxima
      : updateColumnMaximaIncremental(prev.maxima, prev.flatRows, prev.rowIndex, flatRows, rowIndex, columns.columns, model.changedKeys))
    : computeColumnMaxima(flatRows, columns.columns);
  const dataKeys = flatRows.filter(item => item.kind === 'data').map(item => item.key);
  const forward = prev !== null && step.speed > 0 && model.hour > prev.model.hour && prev.sortSig === sortSig;
  const markers = rankMarkersOn && forward
    ? rankMarkers(prev.orderKeys, dataKeys, step.now, new Map(prev.markers), rankHoldForSpeed(step.speed))
    : new Map<string, RankMarker>();
  const glideMs = glideDurationForSpeed(step.speed, binding.reducedMotion === true);
  const glide = new Map<string, GlideOffset>();
  if (prev && forward && glideMs > 0 && !panes && !prev.panes) {
    const oldIndex = new Map<string, number>();
    prev.flatRows.forEach((item, index) => { if (item.kind === 'data') oldIndex.set(item.key, index); });
    flatRows.forEach((item, index) => {
      if (item.kind !== 'data') return;
      const before = oldIndex.get(item.key);
      if (before !== undefined && before !== index) glide.set(item.key, { from: (before - index) * rowHeight, to: 0, started: step.now });
      else {
        const existing = prev.glide.get(item.key);
        if (existing && step.now - existing.started < glideMs) glide.set(item.key, existing);
      }
    });
  }
  const shape: TableShape = {
    model, options: binding.options, settings, expanded, width, theme,
    flatRows, rowIndex, hasGroups, incremental: inc !== null,
    hasTree: inc === null && flatRows.some(item => item.kind === 'data' && item.isTreeGroup),
    panes, paneGeom, rankShown, rankMarkersOn, rankWidth,
    widthKey, fixedWidths, allLayouts, columns, rowHeightMode, rowHeight, rowGeom,
    windowOffset, totalCount, totalHeight, maxima, maximaCols,
    rowAuditCache: inc && prev ? prev.rowAuditCache : new WeakMap(),
    sortSig, orderKeys: dataKeys, markers, glide,
  };
  shapes.set(model, shape);
  return shape;
}

function layoutTable(model: TableModel, binding: FamilyTableBinding, surface: Surface, theme: FamilyTheme, measure: OffscreenCanvasRenderingContext2D, _hour: number, step: LaneStep<TableModel, TableLayout>): TableLayout {
  const shape = shapeOf(model, binding, theme, surface.width, measure, step);
  const { panes, rowGeom, rowHeight, flatRows } = shape;
  const windowTop = shape.windowOffset * rowHeight;
  const localScrollTop = Math.max(0, binding.scrollTop - windowTop);
  const spans = panes ? combinedPaneGeometry(panes) : null;
  const rowWindow = spans ? { first: 0, count: flatRows.length } : tableRowWindow(flatRows.length, rowGeom, rowHeight, localScrollTop, surface.height);
  const tops = (index: number) => (spans ? spans.tops[index]! : (rowGeom ? rowGeom.tops[index]! : index * rowHeight) + windowTop);
  const heights = (index: number) => (spans ? spans.heights[index]! : rowGeom ? rowGeom.heights[index]! : rowHeight);
  const focused = binding.focusedKey;
  const focusedIndex = focused ? (panes ? (paneOfRow(panes, focused)?.index ?? -1) : (shape.rowIndex.get(focused) ?? -1)) : -1;
  return { shape, localScrollTop, rowWindow, tops, heights, focusedIndex, height: surface.height };
}

function liveGlide(frame: TableFrame): { readonly duration: number; readonly glide: ReadonlyMap<string, GlideOffset> } {
  const duration = glideDurationForSpeed(frame.speed, frame.binding.reducedMotion === true);
  const all = frame.layout.shape.glide;
  if (duration === 0 || all.size === 0) return { duration, glide: new Map() };
  const live = new Map<string, GlideOffset>();
  for (const [key, g] of all) if (frame.now - g.started < duration) live.set(key, g);
  return { duration, glide: live };
}

function liveMarkers(frame: TableFrame): Map<string, RankMarker> {
  const live = new Map<string, RankMarker>();
  for (const [key, m] of frame.layout.shape.markers) if (m.until > frame.now) live.set(key, m);
  return live;
}

function visibleRange(layout: TableLayout, gliding: boolean): readonly [number, number] {
  const { shape, localScrollTop, height } = layout;
  const { rowGeom, rowHeight, flatRows } = shape;
  const extra = gliding ? 2 : 0;
  if (rowGeom) {
    let lo = 0;
    let hi = flatRows.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (rowGeom.tops[mid]! + rowGeom.heights[mid]! <= localScrollTop) lo = mid + 1;
      else hi = mid;
    }
    let end = lo;
    while (end < flatRows.length && rowGeom.tops[end]! < localScrollTop + height) end++;
    return [lo, Math.min(flatRows.length, end + extra)];
  }
  return [Math.max(0, Math.floor(localScrollTop / rowHeight)), Math.min(flatRows.length, Math.ceil((localScrollTop + height) / rowHeight) + extra)];
}

const inspectedRow = (inspect: string | null): string | null => (inspect?.startsWith('row:') ? inspect.slice(4) : null);

export const tableLane: LaneFamily<FamilyTableBinding, TableModel, TableLayout> = {
  family: 'table',
  boundary: nextTableChange,
  redraw: () => Infinity,
  status(model) {
    if (model.refused !== undefined) return { refused: model.refused };
    const handled = model.rows.length > 0 || model.panes !== undefined || (model.slots?.length ?? 0) > 0 || model.empty !== undefined || model.clockSummary !== undefined;
    return handled ? 'picture' : 'scene';
  },
  view: ['settings', 'expanded', 'scrollTop', 'scrollLeft', 'focusedKey', 'reducedMotion'],
  layout: layoutTable,
  paint(ctx, frame) {
    const { layout, binding, model, theme } = frame;
    const shape = layout.shape;
    const { duration, glide } = liveGlide(frame);
    const markers = liveMarkers(frame);
    const list = picturesFor(ctx, frame.surface, frame.art.version);
    const common = {
      layout: shape.columns,
      settings: binding.settings,
      hour: model.hour,
      height: layout.height,
      theme,
      now: frame.now,
      inspectedKey: inspectedRow(frame.inspect),
      selectedKey: binding.selectedKey ?? null,
      focusedKey: binding.focusedKey ?? null,
      icons: frame.art.map,
      iconVersion: frame.art.version,
      artCache: frame.art,
      dpr: frame.surface.dpr > 0 ? frame.surface.dpr : 1,
      precomputedMaxima: shape.maxima,
      staticRowLooks: model.rowTimeBoundaries === false,
      rowHeightMode: shape.rowHeightMode,
    };
    if (shape.panes && shape.paneGeom) {
      const scroll = paneScroll(shape.columns, shape.paneGeom, binding.scrollLeft);
      let drew = false;
      for (let p = 0; p < shape.panes.length; p++) {
        const pane = shape.panes[p]!;
        const picture = list[p] ?? (list[p] = createTablePicture());
        ctx.save();
        ctx.translate(shape.paneGeom.xs[p]!, 0);
        ctx.beginPath();
        ctx.rect(0, 0, shape.paneGeom.width, layout.height);
        ctx.clip();
        const result = drawFamilyTable(ctx, {
          ...common,
          rows: pane.rows,
          width: shape.paneGeom.width,
          scrollLeft: scroll.left,
          scrollTop: binding.scrollTop,
          rankShown: false,
          rankWidth: 0,
          glideDuration: 0,
          glide,
          hasGroupHeaders: true,
          rowIndex: pane.rowIndex,
          rowGeom: pane.geom,
        }, picture);
        ctx.restore();
        drew = drew || result.drew;
      }
      if (drew) drawPaneFrame(ctx, shape.paneGeom, shape.columns, scroll, layout.height, theme);
    } else {
      const picture = list[0] ?? (list[0] = createTablePicture());
      drawFamilyTable(ctx, {
        ...common,
        rows: shape.flatRows,
        width: frame.surface.width,
        scrollLeft: binding.scrollLeft,
        scrollTop: layout.localScrollTop,
        rankShown: shape.rankShown,
        rankWidth: shape.rankWidth,
        glideDuration: duration,
        glide,
        markers,
        empty: model.empty,
        hasGroupHeaders: shape.hasGroups,
        dateGrouped: model.dateGrouped === true,
        rowIndex: shape.rowIndex,
        rowGeom: shape.rowGeom,
        clockSummary: model.clockSummary,
      }, picture);
    }
    if (duration > 0 && glide.size > 0) return true;
    let until = Infinity;
    for (const m of markers.values()) if (m.until < until) until = m.until;
    return Number.isFinite(until) ? until : false;
  },
  hit(frame, point) {
    const { model, layout, binding } = frame;
    const shape = layout.shape;
    if (model.clockSummary) return null;
    if (shape.panes && shape.paneGeom) {
      return hitPanes(shape.columns, shape.panes, shape.paneGeom, { x: point[0], y: point[1] }, shape.rowHeight, paneScroll(shape.columns, shape.paneGeom, binding.scrollLeft), binding.scrollTop);
    }
    const { duration, glide } = liveGlide(frame);
    const at = hitFamilyTable(shape.columns, shape.flatRows, { x: point[0], y: point[1] }, shape.rowHeight, binding.scrollLeft, layout.localScrollTop, shape.rankShown, shape.rankWidth, glide.size ? glide : undefined, frame.now, duration, shape.rowGeom, shape.rowIndex, layout.height);
    if (at.kind !== 'row' || !at.rowKey) return null;
    return at.toggle ? `toggle:${at.rowKey}` : `row:${at.rowKey}`;
  },
  tooltip(frame, key, focus) {
    const rowKey = focus ? key : inspectedRow(key);
    if (!rowKey) return null;
    const content = deriveTableTooltip(frame.layout.shape.flatRows, rowKey, frame.model.columns);
    return content ? { hour: frame.model.hour, rowKey, content } : null;
  },
  structure({ model, layout, binding }) {
    const shape = layout.shape;
    const { columns, allLayouts, paneGeom, panes, flatRows } = shape;
    const fields: Record<string, unknown> = {
      widths: {
        widths: Object.fromEntries([...allLayouts.filter(col => col.shown), ...columns.columns].map(col => [col.key, col.width])),
        visible: columns.columns.map(col => col.key),
        totalWidth: paneGeom ? binding.width + Math.max(0, columns.totalWidth - paneGeom.width) : columns.totalWidth,
        scrollable: columns.scrollable,
        pinnedCount: columns.pinnedCount,
      },
      meta: {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        summaryText: model.titleSummary ?? null,
        summaryPartial: model.titleSummaryPartial ?? null,
        hideHeader: model.hideHeader === true,
        rowHeightMode: shape.rowHeightMode,
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
        hasTree: shape.hasTree,
        unsorted: keepsProductOrder(model),
        rankShown: shape.rankShown,
        rankWidth: shape.rankWidth,
        columns: model.columns,
      },
      nav: flatRows.flatMap((item, i) => (item.kind === 'data'
        ? [[item.key, item.name, i, item.isTreeGroup ? (item.isOpen ? 1 : 0) : -1, item.parent ?? '', item.row.meta ?? null, layout.tops(i), layout.heights(i)] as const]
        : [])),
      table: { count: shape.totalCount, totalHeight: shape.totalHeight, focusedIndex: layout.focusedIndex },
      range: layout.rowWindow,
    };
    if (panes && paneGeom) {
      fields.panes = {
        width: paneGeom.width,
        gutter: paneGeom.gutter,
        xs: paneGeom.xs,
        tableWidth: columns.totalWidth,
        titles: panes.map(pane => ({ key: pane.key, name: paneTitleText(pane.title), missing: pane.title.name.kind === 'notRecorded', flag: pane.title.flag ?? null, flagPending: pane.title.flagPending === true, country: pane.title.country ?? null })),
      };
    }
    return fields;
  },
  mirror({ model, layout, binding }) {
    const shape = layout.shape;
    const visibleSpecs = shape.columns.columns.map(col => model.columns.find(spec => spec.key === col.key)!).filter(Boolean);
    return deriveTableMirror(shape.flatRows, visibleSpecs, shape.rankShown, layout.rowWindow, binding.focusedKey ?? null, layout.focusedIndex);
  },
  audit({ model, layout }) {
    const shape = layout.shape;
    return deriveTableAudit({
      model,
      flatRows: shape.flatRows,
      visibleColumns: shape.columns.columns,
      rankShown: shape.rankShown,
      maxima: shape.maxima,
      window: layout.rowWindow,
      rowAuditCache: shape.rowAuditCache,
    });
  },
  visibleArt(frame) {
    const { layout, binding } = frame;
    const shape = layout.shape;
    if (shape.panes) {
      return collectArtUrls(shape.panes.flatMap(pane => {
        const [first, last] = visiblePaneRange(pane.geom, pane.rows.length, binding.scrollTop, layout.height);
        return visibleCellArtItems(pane.rows, shape.columns.columns, first, last);
      }));
    }
    const [first, last] = visibleRange(layout, liveGlide(frame).glide.size > 0);
    return collectArtUrls(visibleCellArtItems(shape.flatRows, shape.columns.columns, first, last));
  },
};

function visibleCellArtItems(
  rows: readonly FlatTableRow[],
  columns: readonly Pick<FamilyColumnLayout, 'key' | 'display' | 'skillIcon'>[],
  first: number,
  last: number,
): ArtUrlInput[] {
  const items: ArtUrlInput[] = [];
  const start = Math.max(0, first);
  const end = Math.min(rows.length, last);
  for (let c = 0; c < columns.length; c++) {
    const col = columns[c]!;
    if (col.skillIcon) items.push(col.skillIcon);
  }
  for (let i = start; i < end; i++) {
    const item = rows[i]!;
    if (item.kind !== 'data') continue;
    for (let c = 0; c < columns.length; c++) {
      const col = columns[c]!;
      const cell = item.row.cells[col.key];
      if (!cell) continue;
      if (cell.flags) {
        for (const fl of cell.flags) if (fl.icon) items.push(fl.icon);
      }
      if (cell.icons) {
        for (const ic of cell.icons) if (ic.url) items.push(ic.url);
      }
      if (cell.icon) items.push(cell.icon);
      if (cell.unitIcon) items.push(cell.unitIcon);
      if (cell.iconSlot && showsIcon(col.display)) {
        const resolved = resolveCell(cell);
        if (resolved.icon) items.push(resolved.icon);
      }
    }
  }
  return items;
}
