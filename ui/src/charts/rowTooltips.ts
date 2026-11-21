
export const isMenuKey = (event: Pick<KeyboardEvent, 'key' | 'shiftKey'>) => event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey);

export const rowPoint = (rect: Pick<DOMRect, 'right' | 'top' | 'height'>) => ({ x: rect.right - 8, y: rect.top + rect.height / 2 });

export const rowDescriptionId = (owner: string, key: string | number) => `row-${encodeURIComponent(JSON.stringify([owner, typeof key, String(key)]))}-description`;
