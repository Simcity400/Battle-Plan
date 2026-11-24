import { drawScrollFade, resolveKitTokens } from '../../draw/kit.ts';
import { formatValueState } from '../valueFormat.ts';
import type { FamilyTableLayout } from './layout.ts';
import { resolveTableRows, type FlatTableRow, type TableModel, type TablePane, type TablePaneTitle } from './model.ts';
import { computeRowLayout, hitFamilyTable, type RowLayoutGeometry } from './painter.ts';
import type { TableCanvasContext, TableSettings, TableTheme } from './settings.ts';

export const PANE_GUTTER = 28;
export const PANE_INDICATOR_HEIGHT = 3;
export const PANE_FADE_WIDTH = 24;

export interface PaneGeometry {
  readonly view: number;
  readonly width: number;
  readonly xs: readonly number[];
  readonly gutter: number;
}

export interface ResolvedPane {
  readonly key: string;
  readonly title: TablePaneTitle;
  readonly rows: readonly FlatTableRow[];
  readonly rowIndex: ReadonlyMap<string, number>;
  readonly geom: RowLayoutGeometry;
}

export interface PaneScroll {
  readonly left: number;
  readonly overflow: number;
}

export function paneGeometry(viewWidth: number, count: number): PaneGeometry {
  const n = Math.max(1, count);
  const width = Math.max(1, Math.floor((viewWidth - PANE_GUTTER * (n - 1)) / n));
  const xs: number[] = [];
  for (let i = 0; i < n; i++) xs.push(i * (width + PANE_GUTTER));
  return { view: viewWidth, width, xs, gutter: PANE_GUTTER };
}

export function paneScroll(layout: FamilyTableLayout, geometry: PaneGeometry, scrollLeft: number): PaneScroll {
  const overflow = Math.max(0, layout.totalWidth - geometry.width);
  return { left: Math.max(0, Math.min(overflow, scrollLeft)), overflow };
}

export function paneModel(model: TableModel, pane: TablePane): TableModel {
  const { panes: _panes, ...rest } = model;
  return { ...rest, rows: pane.rows, ...(pane.totalRow ? { totalRow: pane.totalRow } : { totalRow: undefined }) };
}

export function paneTitleText(title: TablePaneTitle): string {
  const formatted = formatValueState(title.name);
  return formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' ? formatted.label : formatted.text;
}

export function resolvePanes(
  model: TableModel,
  settings: TableSettings,
  expanded: Readonly<Record<string, boolean>> | undefined,
  rowHeightMode: 'standard' | 'media',
): ResolvedPane[] {
  const panes = model.panes ?? [];
  const seen = new Set<string>();
  return panes.map(pane => {
    const rows = resolveTableRows(paneModel(model, pane), settings, expanded);
    const rowIndex = new Map<string, number>();
    for (let i = 0; i < rows.length; i++) {
      const item = rows[i]!;
      if (item.kind === 'group-header' || item.kind === 'loading') continue;
      if (seen.has(item.key)) throw new Error(`table pane ${pane.key}: row key ${item.key} is already in another pane`);
      seen.add(item.key);
      rowIndex.set(item.key, i);
    }
    return { key: pane.key, title: pane.title, rows, rowIndex, geom: computeRowLayout(rows, settings.density, rowHeightMode) };
  });
}

export function combinedPaneRows(panes: readonly ResolvedPane[]): FlatTableRow[] {
  const out: FlatTableRow[] = [];
  for (const pane of panes) {
    let count = 0;
    for (const item of pane.rows) if (item.kind === 'data' && !item.isTotal) count++;
    out.push({ kind: 'group-header', key: `pane:${pane.key}`, label: paneTitleText(pane.title), count });
    for (const item of pane.rows) out.push(item);
  }
  return out;
}

export function panesHeight(panes: readonly ResolvedPane[]): number {
  let height = 0;
  for (const pane of panes) height = Math.max(height, pane.geom.totalHeight);
  return height;
}

export function paneOfRow(panes: readonly ResolvedPane[], key: string): { readonly pane: number; readonly index: number } | null {
  for (let p = 0; p < panes.length; p++) {
    const index = panes[p]!.rowIndex.get(key);
    if (index !== undefined) return { pane: p, index };
  }
  return null;
}

export function visiblePaneRange(geom: RowLayoutGeometry, count: number, scrollTop: number, height: number): readonly [number, number] {
  let lo = 0;
  let hi = count;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (geom.tops[mid]! + geom.heights[mid]! <= scrollTop) lo = mid + 1;
    else hi = mid;
  }
  let end = lo;
  while (end < count && geom.tops[end]! < scrollTop + height) end++;
  return [lo, end];
}

export function hitPanes(
  layout: FamilyTableLayout,
  panes: readonly ResolvedPane[],
  geometry: PaneGeometry,
  point: { readonly x: number; readonly y: number },
  rowHeight: number,
  scroll: PaneScroll,
  scrollTop: number,
): string | null {
  for (let p = 0; p < panes.length; p++) {
    const x0 = geometry.xs[p]!;
    if (point.x < x0 || point.x >= x0 + geometry.width) continue;
    const pane = panes[p]!;
    const at = hitFamilyTable(layout, pane.rows, { x: point.x - x0, y: point.y }, rowHeight, scroll.left, scrollTop, false, 0, undefined, undefined, undefined, pane.geom, pane.rowIndex);
    if (at.kind !== 'row' || !at.rowKey) return null;
    return at.toggle ? `toggle:${at.rowKey}` : `row:${at.rowKey}`;
  }
  return null;
}

export function drawPaneFrame(
  ctx: TableCanvasContext,
  geometry: PaneGeometry,
  layout: FamilyTableLayout,
  scroll: PaneScroll,
  height: number,
  theme: TableTheme,
): void {
  const tokens = resolveKitTokens(theme);
  for (let p = 1; p < geometry.xs.length; p++) {
    const gutterLeft = geometry.xs[p - 1]! + geometry.width;
    ctx.fillStyle = theme.surface;
    ctx.fillRect(gutterLeft, 0, geometry.gutter, height);
    ctx.fillStyle = theme.hairlineStrong ?? theme.line;
    ctx.fillRect(Math.round(gutterLeft + geometry.gutter / 2), 0, 1, height);
  }
  const usedRight = geometry.xs[geometry.xs.length - 1]! + geometry.width;
  if (geometry.view > usedRight) {
    ctx.fillStyle = theme.surface;
    ctx.fillRect(usedRight, 0, geometry.view - usedRight, height);
  }
  if (scroll.overflow <= 0) return;
  const bodyWidth = Math.max(1, geometry.width - layout.pinnedWidth);
  const thumbWidth = Math.max(16, Math.round(bodyWidth * (geometry.width - layout.pinnedWidth) / Math.max(1, layout.totalWidth - layout.pinnedWidth)));
  const thumbX = Math.round((bodyWidth - thumbWidth) * (scroll.left / scroll.overflow));
  const y = height - PANE_INDICATOR_HEIGHT - 2;
  for (const x0 of geometry.xs) {
    const fadeRight = x0 + geometry.width;
    if (scroll.left < scroll.overflow - 0.5) drawScrollFade(ctx, fadeRight - PANE_FADE_WIDTH, 0, PANE_FADE_WIDTH, height, theme, 'right');
    const trackX = x0 + layout.pinnedWidth;
    ctx.fillStyle = tokens.barTrack;
    ctx.beginPath();
    ctx.roundRect(trackX, y, bodyWidth, PANE_INDICATOR_HEIGHT, PANE_INDICATOR_HEIGHT / 2);
    ctx.fill();
    ctx.fillStyle = theme.muted;
    ctx.beginPath();
    ctx.roundRect(trackX + thumbX, y, thumbWidth, PANE_INDICATOR_HEIGHT, PANE_INDICATOR_HEIGHT / 2);
    ctx.fill();
  }
}

