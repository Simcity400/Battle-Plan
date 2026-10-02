/** Rows (bars, table rows) whose hover and keyboard focus show a tooltip and whose
 * right-click or context-menu key opens a menu, the same way in every widget. Pure
 * helpers; `useRowTooltips` wires them to a list of rows. */

/** Whether a key opens the context menu: the menu key, or Shift+F10. */
export const isMenuKey = (event: Pick<KeyboardEvent, 'key' | 'shiftKey'>) => event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey);

/** Where a focused row's tooltip and menu open: just inside its right edge, halfway down. */
export const rowPoint = (rect: Pick<DOMRect, 'right' | 'top' | 'height'>) => ({ x: rect.right - 8, y: rect.top + rect.height / 2 });

/** One whitespace-free id per owner and typed key, including arbitrary content text. */
export const rowDescriptionId = (owner: string, key: string | number) => `row-${encodeURIComponent(JSON.stringify([owner, typeof key, String(key)]))}-description`;
