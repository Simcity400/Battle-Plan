/** The key that shows or hides map diagnostics, here and in the native map. */
export const DIAGNOSTICS_KEY = 'F3';

/** Whether a key press is the diagnostics shortcut: F3 alone, not held down. */
export function isDiagnosticsShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'altKey' | 'metaKey' | 'shiftKey' | 'repeat' | 'defaultPrevented'>): boolean {
  return !event.defaultPrevented && !event.repeat && event.key === DIAGNOSTICS_KEY
    && !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey;
}
