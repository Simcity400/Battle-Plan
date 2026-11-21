export const DIAGNOSTICS_KEY = 'F3';

export function isDiagnosticsShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'altKey' | 'metaKey' | 'shiftKey' | 'repeat' | 'defaultPrevented'>): boolean {
  return !event.defaultPrevented && !event.repeat && event.key === DIAGNOSTICS_KEY
    && event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey;
}
