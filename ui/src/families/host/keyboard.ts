import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import type { MirrorModel } from '../contract.ts';
import { extentTop, type RowExtents } from '../table/layout.ts';
import type { FamilyRowWindow } from './adapter.ts';

export const TYPEAHEAD_RESET_MS = 700;

export type FamilyNavTuple = readonly [key: string, name: string, index: number, treeState: number, parent: string, meta?: unknown, top?: number, height?: number];

export interface TableOrdinalMetrics {
  readonly count: number;
  readonly totalHeight: number;
  readonly extents?: RowExtents;
  readonly gapBefore?: readonly number[];
}

function isGapOrdinal(ordinal: number, gapBefore: readonly number[] | undefined): boolean {
  if (!gapBefore?.length) return false;
  let lo = 0;
  let hi = gapBefore.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    const at = gapBefore[mid]! + mid;
    if (at < ordinal) lo = mid + 1;
    else if (at > ordinal) hi = mid;
    else return true;
  }
  return false;
}

export function nearestDataOrdinal(
  ordinal: number,
  totalCount: number,
  gapBefore: readonly number[] | undefined,
  direction: 1 | -1,
): number | null {
  if (totalCount <= 0) return null;
  const clamped = Math.max(0, Math.min(totalCount - 1, ordinal));
  if (!gapBefore?.length) return clamped;
  let at = clamped;
  while (at >= 0 && at < totalCount && isGapOrdinal(at, gapBefore)) at += direction;
  if (at >= 0 && at < totalCount) return at;
  at = clamped - direction;
  while (at >= 0 && at < totalCount && isGapOrdinal(at, gapBefore)) at -= direction;
  return at >= 0 && at < totalCount ? at : null;
}

export function tableOrdinalBounds(
  table: TableOrdinalMetrics,
  nav: readonly FamilyNavTuple[],
  ordinal: number,
): { readonly top: number; readonly height: number } | null {
  if (table.count <= 0 || ordinal < 0 || ordinal >= table.count) return null;
  if (table.extents) {
    const top = extentTop(table.extents, table.extents.rowHeight, ordinal);
    const height = extentTop(table.extents, table.extents.rowHeight, ordinal + 1) - top;
    return { top, height };
  }
  const sampleHeight = nav[0]?.[7];
  const rowHeight = sampleHeight && sampleHeight > 0 ? sampleHeight : table.totalHeight / table.count;
  if (!(rowHeight > 0)) return null;
  return { top: ordinal * rowHeight, height: rowHeight };
}

export function tablePageSteps(
  table: TableOrdinalMetrics,
  nav: readonly FamilyNavTuple[],
  currentOrdinal: number,
  viewportHeight: number,
): FamilyPageSteps {
  const count = table.count;
  if (count <= 1 || viewportHeight <= 0) return { down: 1, up: 1 };
  const sampleHeight = nav[0]?.[7];
  const rowHeight = table.extents
    ? table.extents.rowHeight
    : (sampleHeight && sampleHeight > 0 ? sampleHeight : table.totalHeight / count);
  if (!(rowHeight > 0)) return { down: 1, up: 1 };
  const at = Math.max(0, Math.min(count - 1, currentOrdinal));
  const extents = table.extents ?? null;
  const topOf = (ord: number) => (extents ? extentTop(extents, rowHeight, ord) : ord * rowHeight);
  const reach = topOf(at) + viewportHeight;
  let loDown = at;
  let hiDown = count - 1;
  while (loDown < hiDown) {
    const mid = (loDown + hiDown + 1) >>> 1;
    if (topOf(mid + 1) <= reach) loDown = mid;
    else hiDown = mid - 1;
  }
  const floor = topOf(at + 1) - viewportHeight;
  let loUp = 0;
  let hiUp = at;
  while (loUp < hiUp) {
    const mid = (loUp + hiUp) >>> 1;
    if (topOf(mid) < floor) loUp = mid + 1;
    else hiUp = mid;
  }
  return { down: Math.max(1, loDown - at), up: Math.max(1, at - loUp) };
}

export function resolveInstalledOrdinal(
  nav: readonly FamilyNavTuple[],
  ordinal: number,
  table: TableOrdinalMetrics,
  requested: FamilyRowWindow | null,
  mirror: MirrorModel | undefined,
  actionKeys: ReadonlyMap<string, unknown>,
): FamilyNavTuple | null {
  const exact = nav.find(item => item[2] === ordinal);
  if (exact) return exact;
  if (
    requested
    && requested.first + requested.count >= table.count
    && nav.length > 0
    && nav[0]![2] >= requested.first
    && ordinal > nav[nav.length - 1]![2]
    && mirror
    && !mirror.rows.some(row => !row.groupHeader && row.name === 'Loading' && !actionKeys.has(row.key))
  ) {
    return nav[nav.length - 1]!;
  }
  return null;
}

export interface FamilyKeyboardOptions {
  readonly nav: readonly FamilyNavTuple[];
  readonly totalCount?: number;
  readonly focusedKey: string | null;
  readonly focusedIndex?: number;
  readonly pageStep: FamilyPageSteps;
  readonly blocked: boolean;
  readonly typeaheadRef?: { current: { query: string; until: number } };
  readonly isPinnedTooltip: () => boolean;
  readonly onFocusKey: (key: string) => void;
  readonly onFocusOrdinal?: (ordinal: number, direction?: 1 | -1) => void;
  readonly onActivate: (entry: FamilyNavTuple) => void;
  readonly onToggleTree?: (key: string) => void;
  readonly onContextMenu?: (entry: FamilyNavTuple | null) => boolean;
  readonly onTogglePinTooltip?: (activeKey: string | null) => void;
  readonly onUnpinTooltip?: () => void;
  readonly onEscape?: () => void;
}

export function usableRowHeight(fields: Readonly<Record<string, unknown>>, viewportHeight: number): number {
  const rows = (fields.meta as { readonly rowViewport?: { readonly height: number } } | undefined)?.rowViewport;
  return rows ? rows.height : viewportHeight;
}

export interface FamilyPageSteps {
  readonly down: number;
  readonly up: number;
}

export function navPageSteps(nav: readonly FamilyNavTuple[], focusedKey: string | null, viewportHeight: number): FamilyPageSteps {
  const at = Math.max(0, nav.findIndex(entry => entry[0] === focusedKey));
  const top = nav[at]?.[6];
  if (top === undefined) return { down: Math.max(1, nav.length), up: Math.max(1, nav.length) };
  const reach = top + viewportHeight;
  let down = 0;
  for (let i = at + 1; i < nav.length; i++) {
    const entry = nav[i]!;
    if (entry[6] === undefined || entry[6] + (entry[7] ?? 0) > reach) break;
    down++;
  }
  const floor = top + (nav[at]![7] ?? 0) - viewportHeight;
  let up = 0;
  for (let i = at - 1; i >= 0; i--) {
    const entry = nav[i]!;
    if (entry[6] === undefined || entry[6] < floor) break;
    up++;
  }
  return { down: Math.max(1, down), up: Math.max(1, up) };
}

export function handleFamilyKeyDown(event: ReactKeyboardEvent | KeyboardEvent, options: FamilyKeyboardOptions): void {
  const { nav, pageStep, blocked } = options;
  const totalCount = options.totalCount ?? nav.length;

  if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && !nav.length) {
    if (blocked) return;
    if (options.onContextMenu?.(null)) event.preventDefault();
    return;
  }

  if (!nav.length && (!options.onFocusOrdinal || totalCount <= 0)) {
    return;
  }

  const matchedIdx = options.focusedKey !== null ? nav.findIndex(item => item[0] === options.focusedKey) : -1;
  const hasExplicitFocus = matchedIdx >= 0;
  const idx = hasExplicitFocus ? matchedIdx : (nav.length > 0 ? 0 : -1);
  const entry = idx >= 0 ? nav[idx] : undefined;
  const hasCurrentOrdinal = hasExplicitFocus || (options.focusedIndex !== undefined && options.focusedIndex >= 0);
  const currentOrdinal = hasExplicitFocus ? nav[matchedIdx]![2] : (options.focusedIndex ?? 0);

  const moveBy = (delta: number) => {
    if (options.onFocusOrdinal) {
      const nextOrdinal = hasCurrentOrdinal ? currentOrdinal + delta : 0;
      const clamped = Math.max(0, Math.min(Math.max(0, totalCount - 1), nextOrdinal));
      options.onFocusOrdinal(clamped, delta < 0 ? -1 : 1);
      return;
    }
    if (!nav.length) return;
    const clamped = Math.max(0, Math.min(nav.length - 1, idx + delta));
    options.onFocusKey(nav[clamped]![0]);
  };

  if (event.key === 'ArrowDown') {
    event.preventDefault();
    moveBy(1);
    return;
  }
  if (event.key === 'ArrowUp') {
    event.preventDefault();
    moveBy(-1);
    return;
  }
  if (event.key === 'Home') {
    event.preventDefault();
    if (options.onFocusOrdinal) options.onFocusOrdinal(0, 1);
    else if (nav.length) options.onFocusKey(nav[0]![0]);
    return;
  }
  if (event.key === 'End') {
    event.preventDefault();
    if (options.onFocusOrdinal) options.onFocusOrdinal(Math.max(0, totalCount - 1), -1);
    else if (nav.length) options.onFocusKey(nav[nav.length - 1]![0]);
    return;
  }
  if (event.key === 'PageDown') {
    event.preventDefault();
    moveBy(pageStep.down);
    return;
  }
  if (event.key === 'PageUp') {
    event.preventDefault();
    moveBy(-pageStep.up);
    return;
  }
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    const target = hasExplicitFocus ? nav[matchedIdx]! : undefined;
    if (!target || blocked) return;
    if (target[3] >= 0 && options.onToggleTree) options.onToggleTree(target[0]);
    else options.onActivate(target);
    return;
  }
  if (event.key === 'ArrowRight' && hasExplicitFocus && entry && entry[3] === 0 && options.onToggleTree) {
    event.preventDefault();
    options.onToggleTree(entry[0]);
    return;
  }
  if (event.key === 'ArrowLeft' && hasExplicitFocus && entry) {
    if (entry[3] === 1 && options.onToggleTree) {
      event.preventDefault();
      options.onToggleTree(entry[0]);
      return;
    }
    if (entry[4]) {
      event.preventDefault();
      options.onFocusKey(entry[4]);
      return;
    }
  }
  if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
    if (blocked) return;
    const target = hasExplicitFocus ? nav[matchedIdx]! : null;
    if (options.onContextMenu?.(target)) event.preventDefault();
    return;
  }
  if ((event.key === 'p' || event.key === 'P') && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    if (options.isPinnedTooltip()) {
      options.onUnpinTooltip?.();
      return;
    }
    const targetKey = hasExplicitFocus ? nav[matchedIdx]![0] : null;
    options.onTogglePinTooltip?.(targetKey);
    return;
  }
  if (event.key === 'Escape') {
    if (options.isPinnedTooltip()) {
      event.preventDefault();
      options.onUnpinTooltip?.();
      return;
    }
    options.onEscape?.();
    return;
  }
  if (options.typeaheadRef && nav.length && event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
    const now = performance.now();
    const query = now <= options.typeaheadRef.current.until
      ? `${options.typeaheadRef.current.query}${event.key.toLowerCase()}`
      : event.key.toLowerCase();
    options.typeaheadRef.current = { query, until: now + TYPEAHEAD_RESET_MS };
    for (let offsetIdx = 1; offsetIdx <= nav.length; offsetIdx++) {
      const candidate = nav[(Math.max(0, idx) + offsetIdx) % nav.length]!;
      if (candidate[1].toLowerCase().startsWith(query)) {
        event.preventDefault();
        options.onFocusKey(candidate[0]);
        break;
      }
    }
  }
}

export interface FamilyGridKeyboardOptions {
  readonly rows: number;
  readonly columns: number;
  readonly current: { readonly row: number; readonly column: number };
  readonly pageStep: number;
  readonly blocked: boolean;
  readonly isPinnedTooltip: () => boolean;
  readonly onFocusCell: (row: number, column: number) => void;
  readonly onActivate: (row: number, column: number) => void;
  readonly onContextMenu?: (row: number, column: number) => boolean;
  readonly onTogglePinTooltip?: (row: number, column: number) => void;
  readonly onUnpinTooltip?: () => void;
  readonly onEscape?: () => void;
}

export function handleFamilyGridKeyDown(event: ReactKeyboardEvent | KeyboardEvent, options: FamilyGridKeyboardOptions): void {
  const { rows, columns, current, pageStep, blocked } = options;
  if (rows <= 0 || columns <= 0) {
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && !blocked) {
      if (options.onContextMenu?.(-1, -1)) event.preventDefault();
    }
    return;
  }
  const r = Math.max(0, Math.min(rows - 1, current.row));
  const c = Math.max(0, Math.min(columns - 1, current.column));
  if (event.key === 'ArrowRight') {
    event.preventDefault();
    options.onFocusCell(r, Math.min(columns - 1, c + 1));
    return;
  }
  if (event.key === 'ArrowLeft') {
    event.preventDefault();
    options.onFocusCell(r, Math.max(0, c - 1));
    return;
  }
  if (event.key === 'ArrowDown') {
    event.preventDefault();
    options.onFocusCell(Math.min(rows - 1, r + 1), c);
    return;
  }
  if (event.key === 'ArrowUp') {
    event.preventDefault();
    options.onFocusCell(Math.max(0, r - 1), c);
    return;
  }
  if (event.key === 'Home') {
    event.preventDefault();
    options.onFocusCell(event.ctrlKey || event.metaKey ? 0 : r, 0);
    return;
  }
  if (event.key === 'End') {
    event.preventDefault();
    options.onFocusCell(event.ctrlKey || event.metaKey ? rows - 1 : r, columns - 1);
    return;
  }
  if (event.key === 'PageDown') {
    event.preventDefault();
    options.onFocusCell(Math.min(rows - 1, r + pageStep), c);
    return;
  }
  if (event.key === 'PageUp') {
    event.preventDefault();
    options.onFocusCell(Math.max(0, r - pageStep), c);
    return;
  }
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    if (!blocked) options.onActivate(r, c);
    return;
  }
  if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
    if (!blocked && options.onContextMenu?.(r, c)) event.preventDefault();
    return;
  }
  if ((event.key === 'p' || event.key === 'P') && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    if (options.isPinnedTooltip()) options.onUnpinTooltip?.();
    else if (!blocked) options.onTogglePinTooltip?.(r, c);
    return;
  }
  if (event.key === 'Escape') {
    if (options.isPinnedTooltip()) {
      event.preventDefault();
      options.onUnpinTooltip?.();
      return;
    }
    options.onEscape?.();
  }
}

export interface FamilyPointsKeyboardOptions {
  readonly nav: readonly FamilyNavTuple[];
  readonly focusedKey: string | null;
  readonly pageStep: number;
  readonly blocked: boolean;
  readonly isPinnedTooltip: () => boolean;
  readonly onFocusKey: (key: string) => void;
  readonly onActivate: (entry: FamilyNavTuple) => void;
  readonly onContextMenu?: (entry: FamilyNavTuple | null) => boolean;
  readonly onTogglePinTooltip?: (activeKey: string | null) => void;
  readonly onUnpinTooltip?: () => void;
  readonly onEscape?: () => void;
}

export function handleFamilyPointsKeyDown(event: ReactKeyboardEvent | KeyboardEvent, options: FamilyPointsKeyboardOptions): void {
  const { nav, pageStep, blocked } = options;
  if (!nav.length) {
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && !blocked) {
      if (options.onContextMenu?.(null)) event.preventDefault();
    } else if (event.key === 'Escape' && options.isPinnedTooltip()) {
      event.preventDefault();
      options.onUnpinTooltip?.();
    }
    return;
  }
  const matchedIdx = options.focusedKey !== null ? nav.findIndex(item => item[0] === options.focusedKey) : -1;
  const idx = matchedIdx >= 0 ? matchedIdx : 0;
  const entry = nav[idx]!;
  const moveBy = (delta: number) => {
    const nextIdx = matchedIdx >= 0 ? Math.max(0, Math.min(nav.length - 1, idx + delta)) : (delta >= 0 ? 0 : nav.length - 1);
    options.onFocusKey(nav[nextIdx]![0]);
  };
  if (event.key === 'ArrowRight') {
    event.preventDefault();
    moveBy(1);
    return;
  }
  if (event.key === 'ArrowLeft') {
    event.preventDefault();
    moveBy(-1);
    return;
  }
  if (event.key === 'Home') {
    event.preventDefault();
    options.onFocusKey(nav[0]![0]);
    return;
  }
  if (event.key === 'End') {
    event.preventDefault();
    options.onFocusKey(nav[nav.length - 1]![0]);
    return;
  }
  if (event.key === 'PageDown') {
    event.preventDefault();
    moveBy(pageStep);
    return;
  }
  if (event.key === 'PageUp') {
    event.preventDefault();
    moveBy(-pageStep);
    return;
  }
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    if (!blocked) options.onActivate(entry);
    return;
  }
  if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
    if (!blocked && options.onContextMenu?.(entry)) event.preventDefault();
    return;
  }
  if ((event.key === 'p' || event.key === 'P') && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    if (options.isPinnedTooltip()) options.onUnpinTooltip?.();
    else if (!blocked) options.onTogglePinTooltip?.(entry[0]);
    return;
  }
  if (event.key === 'Escape') {
    if (options.isPinnedTooltip()) {
      event.preventDefault();
      options.onUnpinTooltip?.();
      return;
    }
    options.onEscape?.();
  }
}
