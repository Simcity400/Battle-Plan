import type { MirrorModel } from '../contract.ts';

export interface TableMirrorSyncOptions {
  readonly selectedKey: string | null;
  readonly rowsFocusable: boolean;
  readonly focusedKeyRef: { current: string | null };
  readonly focusInsideRef: { readonly current: boolean };
  readonly restoringFocusRef: { current: boolean };
  readonly lastMirrorKeyRef: { current: string };
}

function restoreFocus(row: HTMLTableRowElement | null, key: string | null, options: TableMirrorSyncOptions): boolean {
  if (!row) return false;
  if (document.activeElement !== row) {
    options.restoringFocusRef.current = true;
    try {
      row.focus({ preventScroll: true });
    } finally {
      options.restoringFocusRef.current = false;
    }
  }
  if (key === options.focusedKeyRef.current) return false;
  options.focusedKeyRef.current = key;
  return true;
}

export function syncTableMirror(
  tbody: HTMLTableSectionElement | null,
  mirror: MirrorModel | null,
  options: TableMirrorSyncOptions,
): boolean {
  if (!tbody) return false;
  if (!mirror) {
    if (options.lastMirrorKeyRef.current !== '') {
      options.lastMirrorKeyRef.current = '';
      tbody.replaceChildren();
    }
    return false;
  }
  const selected = options.selectedKey ?? '';
  const focusedKey = options.focusedKeyRef.current;
  const hasFocused = focusedKey !== null && mirror.rows.some(r => !r.groupHeader && r.key === focusedKey);
  const activeFocus = hasFocused ? focusedKey : (mirror.rows.find(r => !r.groupHeader)?.key ?? null);
  const key = JSON.stringify([
    options.rowsFocusable,
    mirror.first,
    mirror.total,
    activeFocus,
    selected,
    mirror.rows.map(r => [
      r.rowIndex,
      r.key,
      r.groupHeader ? 1 : 0,
      r.rank ?? null,
      r.level ?? null,
      r.group ? (r.open ? 1 : 0) : -1,
      r.cells.map(c => c.spoken),
    ]),
  ]);
  if (key === options.lastMirrorKeyRef.current) return false;
  options.lastMirrorKeyRef.current = key;
  const wasFocused = options.rowsFocusable && options.focusInsideRef.current && document.activeElement instanceof HTMLElement && tbody.contains(document.activeElement);
  let restore: HTMLTableRowElement | null = null;
  const canReuseInPlace = typeof tbody.children?.length === 'number' && typeof tbody.lastElementChild !== 'undefined';

  if (canReuseInPlace && tbody.children.length > 0) {
    const existing = tbody.children;
    const targetLen = mirror.rows.length;
    while (existing.length > targetLen) {
      tbody.lastElementChild?.remove();
    }
    for (let i = 0; i < targetLen; i++) {
      const row = mirror.rows[i]!;
      let tr = existing[i] as HTMLTableRowElement | undefined;
      if (!tr) {
        tr = document.createElement('tr');
        tbody.append(tr);
      }
      if (tr.dataset.rowKey !== row.key) tr.dataset.rowKey = row.key;
      const rowIndexStr = String(row.rowIndex + 2);
      if (tr.getAttribute('aria-rowindex') !== rowIndexStr) tr.setAttribute('aria-rowindex', rowIndexStr);
      if (row.groupHeader) {
        if (tr.getAttribute('role') !== 'row') tr.setAttribute('role', 'row');
        tr.removeAttribute('tabindex');
        tr.removeAttribute('aria-level');
        tr.removeAttribute('aria-expanded');
        tr.removeAttribute('aria-selected');
        if (tr.children.length !== 1) {
          const td = document.createElement('td');
          td.textContent = row.name;
          tr.replaceChildren(td);
        } else if (tr.children[0]!.textContent !== row.name) {
          tr.children[0]!.textContent = row.name;
        }
        continue;
      }
      if (tr.hasAttribute('role')) tr.removeAttribute('role');
      if (options.rowsFocusable) {
        const nextTab = row.key === activeFocus ? 0 : -1;
        if (tr.tabIndex !== nextTab) tr.tabIndex = nextTab;
      } else if (tr.hasAttribute('tabindex')) {
        tr.removeAttribute('tabindex');
      }
      if (row.level !== undefined) {
        const lvl = String(row.level);
        if (tr.getAttribute('aria-level') !== lvl) tr.setAttribute('aria-level', lvl);
      } else if (tr.hasAttribute('aria-level')) {
        tr.removeAttribute('aria-level');
      }
      if (row.group) {
        const exp = String(row.open === true);
        if (tr.getAttribute('aria-expanded') !== exp) tr.setAttribute('aria-expanded', exp);
      } else if (tr.hasAttribute('aria-expanded')) {
        tr.removeAttribute('aria-expanded');
      }
      if (selected && selected === row.key) {
        if (tr.getAttribute('aria-selected') !== 'true') tr.setAttribute('aria-selected', 'true');
      } else if (tr.hasAttribute('aria-selected')) {
        tr.removeAttribute('aria-selected');
      }
      const hasRank = row.rank !== undefined;
      const expectedCells = (hasRank ? 1 : 0) + row.cells.length;
      while (tr.children.length > expectedCells) tr.lastElementChild?.remove();
      while (tr.children.length < expectedCells) tr.append(document.createElement('td'));
      let ci = 0;
      if (hasRank) {
        const rankText = String(row.rank);
        const td = tr.children[ci++]!;
        if (td.textContent !== rankText) td.textContent = rankText;
      }
      for (let j = 0; j < row.cells.length; j++) {
        const td = tr.children[ci++]!;
        const spoken = row.cells[j]!.spoken;
        if (td.textContent !== spoken) td.textContent = spoken;
      }
      if (wasFocused && row.key === activeFocus) restore = tr;
    }
    return restoreFocus(restore, activeFocus, options);
  }

  const fragment = document.createDocumentFragment();
  mirror.rows.forEach(row => {
    const tr = document.createElement('tr');
    tr.dataset.rowKey = row.key;
    tr.setAttribute('aria-rowindex', String(row.rowIndex + 2));
    if (row.groupHeader) {
      tr.setAttribute('role', 'row');
      const td = document.createElement('td');
      td.textContent = row.name;
      tr.append(td);
      fragment.append(tr);
      return;
    }
    if (options.rowsFocusable) tr.tabIndex = row.key === activeFocus ? 0 : -1;
    if (row.level !== undefined) tr.setAttribute('aria-level', String(row.level));
    if (row.group) tr.setAttribute('aria-expanded', String(row.open === true));
    if (selected && selected === row.key) tr.setAttribute('aria-selected', 'true');
    if (row.rank !== undefined) {
      const rankTd = document.createElement('td');
      rankTd.textContent = String(row.rank);
      tr.append(rankTd);
    }
    for (const cell of row.cells) {
      const td = document.createElement('td');
      td.textContent = cell.spoken;
      tr.append(td);
    }
    if (wasFocused && row.key === activeFocus) restore = tr;
    fragment.append(tr);
  });
  options.restoringFocusRef.current = true;
  try {
    tbody.replaceChildren(fragment);
  } finally {
    options.restoringFocusRef.current = false;
  }
  return restoreFocus(restore, activeFocus, options);
}

export interface ListboxMirrorItem {
  readonly key: string;
  readonly spoken: string;
  readonly role?: 'option' | 'presentation';
  readonly index?: number;
  readonly selected?: boolean;
  readonly description?: string;
}

export interface ListboxMirrorSyncOptions {
  readonly optionIdPrefix: string;
  readonly total: number;
  readonly focusedKey: string | null;
  readonly lastMirrorKeyRef: { current: string };
}

export function optionElementId(prefix: string, key: string): string {
  const encoded = key.replace(/[^A-Za-z0-9-]/g, ch => `_${ch.codePointAt(0)!.toString(16)}_`);
  return `${prefix}-opt-${encoded}`;
}

export function syncListboxMirror(
  listbox: HTMLElement | null,
  items: readonly ListboxMirrorItem[] | null,
  options: ListboxMirrorSyncOptions,
): { readonly activeDescendantId: string | null; readonly focusedPresent: boolean } {
  if (!listbox) return { activeDescendantId: null, focusedPresent: false };
  if (!items || items.length === 0) {
    if (options.lastMirrorKeyRef.current !== '') {
      options.lastMirrorKeyRef.current = '';
      listbox.replaceChildren();
    }
    listbox.removeAttribute('aria-activedescendant');
    return { activeDescendantId: null, focusedPresent: false };
  }

  let activeId: string | null = null;
  let focusedPresent = false;
  const sig = JSON.stringify([
    options.optionIdPrefix,
    options.total,
    options.focusedKey,
    items.map(item => [
      item.key,
      item.spoken,
      item.role ?? 'option',
      item.index ?? -1,
      item.selected === undefined ? -1 : item.selected ? 1 : 0,
      item.description ?? '',
    ]),
  ]);
  if (sig === options.lastMirrorKeyRef.current) {
    for (let i = 0; i < items.length; i++) {
      const item = items[i]!;
      if ((item.role ?? 'option') === 'option' && options.focusedKey !== null && item.key === options.focusedKey) {
        activeId = optionElementId(options.optionIdPrefix, item.key);
        focusedPresent = true;
        break;
      }
    }
    return { activeDescendantId: activeId, focusedPresent };
  }

  options.lastMirrorKeyRef.current = sig;
  const fragment = document.createDocumentFragment();
  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    const role = item.role ?? 'option';
    const el = document.createElement('div');
    el.setAttribute('role', role);
    el.dataset.rowKey = item.key;
    el.textContent = item.spoken;
    if (role === 'option') {
      const id = optionElementId(options.optionIdPrefix, item.key);
      el.id = id;
      if (item.index !== undefined) {
        el.setAttribute('aria-posinset', String(item.index + 1));
        el.setAttribute('aria-setsize', String(options.total));
      }
      const isActive = options.focusedKey !== null && item.key === options.focusedKey;
      const selected = item.selected !== undefined ? item.selected : isActive;
      el.setAttribute('aria-selected', String(selected));
      if (item.description) el.setAttribute('aria-description', item.description);
      if (isActive) {
        activeId = id;
        focusedPresent = true;
      }
    }
    fragment.append(el);
  }
  listbox.replaceChildren(fragment);
  if (activeId) listbox.setAttribute('aria-activedescendant', activeId);
  else listbox.removeAttribute('aria-activedescendant');
  return { activeDescendantId: activeId, focusedPresent };
}

export function reconcileFocusedKey(
  currentKey: string | null,
  keys: readonly string[],
  fallbackToFirst: boolean,
): { readonly key: string | null; readonly present: boolean; readonly index: number } {
  if (currentKey !== null) {
    const idx = keys.indexOf(currentKey);
    if (idx >= 0) return { key: currentKey, present: true, index: idx };
  }
  if (fallbackToFirst && keys.length > 0) {
    return { key: keys[0]!, present: false, index: 0 };
  }
  return { key: null, present: false, index: -1 };
}
