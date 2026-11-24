import type { KeyboardEvent as ReactKeyboardEvent } from 'react';

export const TYPEAHEAD_RESET_MS = 700;

export type FamilyNavTuple = readonly [key: string, name: string, index: number, treeState: number, parent: string, meta?: unknown];

export interface FamilyKeyboardOptions {
  readonly nav: readonly FamilyNavTuple[];
  readonly totalCount?: number;
  readonly focusedKey: string | null;
  readonly focusedIndex?: number;
  readonly fallbackToFirstOnActivate?: boolean;
  readonly pageStep: number;
  readonly blocked: boolean;
  readonly typeaheadRef?: { current: { query: string; until: number } };
  readonly isPinnedTooltip: () => boolean;
  readonly onFocusKey: (key: string) => void;
  readonly onFocusOrdinal?: (ordinal: number) => void;
  readonly onActivate: (entry: FamilyNavTuple) => void;
  readonly onToggleTree?: (key: string) => void;
  readonly onContextMenu?: (entry: FamilyNavTuple | null) => boolean;
  readonly onTogglePinTooltip?: (activeKey: string | null) => void;
  readonly onUnpinTooltip?: () => void;
  readonly onEscape?: () => void;
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

  const fallbackFirst = options.fallbackToFirstOnActivate !== false;
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
      options.onFocusOrdinal(clamped);
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
    if (options.onFocusOrdinal) options.onFocusOrdinal(0);
    else if (nav.length) options.onFocusKey(nav[0]![0]);
    return;
  }
  if (event.key === 'End') {
    event.preventDefault();
    if (options.onFocusOrdinal) options.onFocusOrdinal(Math.max(0, totalCount - 1));
    else if (nav.length) options.onFocusKey(nav[nav.length - 1]![0]);
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
  if (event.key === 'Enter') {
    event.preventDefault();
    const target = hasExplicitFocus ? nav[matchedIdx]! : (fallbackFirst ? entry : undefined);
    if (!target) return;
    if (target[3] >= 0 && options.onToggleTree) options.onToggleTree(target[0]);
    else options.onActivate(target);
    return;
  }
  if (event.key === 'ArrowRight' && entry && entry[3] === 0 && options.onToggleTree) {
    event.preventDefault();
    options.onToggleTree(entry[0]);
    return;
  }
  if (event.key === 'ArrowLeft' && entry) {
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
    const target = hasExplicitFocus ? nav[matchedIdx]! : (fallbackFirst ? (entry ?? null) : null);
    if (options.onContextMenu?.(target)) event.preventDefault();
    return;
  }
  if ((event.key === 'p' || event.key === 'P') && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    if (options.isPinnedTooltip()) {
      options.onUnpinTooltip?.();
      return;
    }
    const targetKey = hasExplicitFocus ? nav[matchedIdx]![0] : (fallbackFirst ? (entry?.[0] ?? null) : null);
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
