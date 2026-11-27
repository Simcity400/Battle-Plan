import { formatValueState } from '../valueFormat.ts';
import { computeRowLayout, type FamilyTableLayout, type RowLayoutGeometry } from './layout.ts';
import { resolveTableRows, type FlatTableRow, type TableModel, type TablePane, type TablePaneTitle } from './model.ts';
import type { TableSettings } from './settings.ts';

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
  return formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' || formatted.kind === 'pending' ? formatted.label : formatted.text;
}

function paneHeaderKey(paneKey: string): string {
  return `pane:${encodeURIComponent(paneKey)}`;
}

function paneRowKey(paneKey: string, localKey: string): string {
  return `pane:${encodeURIComponent(paneKey)}:${localKey}`;
}

export function resolvePanes(
  model: TableModel,
  settings: TableSettings,
  expanded: Readonly<Record<string, boolean>> | undefined,
  rowHeightMode: 'standard' | 'media',
): ResolvedPane[] {
  const panes = model.panes ?? [];
  const seenPanes = new Set<string>();
  const seen = new Set<string>();
  return panes.map(pane => {
    if (seenPanes.has(pane.key)) throw new Error(`table pane ${pane.key}: duplicate pane key`);
    seenPanes.add(pane.key);
    const headerKey = paneHeaderKey(pane.key);
    if (seen.has(headerKey)) throw new Error(`table pane ${pane.key}: header key ${headerKey} is already in another pane`);
    seen.add(headerKey);
    const paneSeen = new Set<string>([headerKey]);
    const raw = resolveTableRows(paneModel(model, pane), settings, expanded);
    const rows: FlatTableRow[] = new Array(raw.length);
    for (let i = 0; i < raw.length; i++) {
      const item = raw[i]!;
      const resolved = item.kind === 'data' ? item : { ...item, key: paneRowKey(pane.key, item.key) };
      if (paneSeen.has(item.key) || paneSeen.has(resolved.key)) throw new Error(`table pane ${pane.key}: duplicate row key ${item.key}`);
      if (seen.has(resolved.key)) throw new Error(`table pane ${pane.key}: row key ${item.key} is already in another pane`);
      paneSeen.add(item.key);
      paneSeen.add(resolved.key);
      seen.add(resolved.key);
      rows[i] = resolved;
    }
    return { key: pane.key, title: pane.title, rows, geom: computeRowLayout(rows, settings.density, rowHeightMode) };
  });
}

export function combinedPaneRows(panes: readonly ResolvedPane[]): FlatTableRow[] {
  const out: FlatTableRow[] = [];
  for (const pane of panes) {
    let count = 0;
    for (const item of pane.rows) if (item.kind === 'data' && !item.isTotal) count++;
    out.push({ kind: 'group-header', key: paneHeaderKey(pane.key), label: paneTitleText(pane.title), count });
    for (const item of pane.rows) out.push(item);
  }
  return out;
}

export function combinedPaneGeometry(panes: readonly ResolvedPane[]): { readonly tops: readonly number[]; readonly heights: readonly number[] } {
  const tops: number[] = [];
  const heights: number[] = [];
  for (const pane of panes) {
    tops.push(0);
    heights.push(0);
    for (let i = 0; i < pane.rows.length; i++) {
      tops.push(pane.geom.tops[i]!);
      heights.push(pane.geom.heights[i]!);
    }
  }
  return { tops, heights };
}

export function panesHeight(panes: readonly ResolvedPane[]): number {
  let height = 0;
  for (const pane of panes) height = Math.max(height, pane.geom.totalHeight);
  return height;
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
