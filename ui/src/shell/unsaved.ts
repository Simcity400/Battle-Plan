export type Leaving = 'close' | 'update';

export const RESTARTING = 'Restarting…';

export function leavingTitle(leaving: Leaving): string {
  return leaving === 'update' ? 'Restarting BattlePlan to update…' : 'Closing BattlePlan…';
}

export interface LeavePrompt { title: string; detail: string; discard: string; keep?: string; hideSave?: boolean }

const listed = (names: readonly string[]) => names.length < 2 ? names.join('') : `${names.slice(0, -1).join(', ')} and ${names.at(-1)}`;

export function unsavedDetail(
  layoutPages: readonly string[],
  theme: string | null,
  writes: 'saving' | 'failed' | null = null,
  exportRunning = false,
): string | null {
  const parts = [layoutPages.length ? `Layout changes on ${listed(layoutPages)}` : null, theme === null ? null : theme ? `the theme “${theme}”` : 'your new theme'].filter(Boolean) as string[];
  const saving = writes === 'saving' ? 'A settings change is still being saved.' : writes === 'failed' ? 'A settings change isn’t saved.' : null;
  const exporting = exportRunning ? 'A video export is still running and will be cancelled if BattlePlan closes.' : null;
  if (!parts.length) {
    return [saving, exporting].filter(Boolean).join(' ') || null;
  }
  const sentence = parts.join(' and ');
  const unsaved = `${sentence[0].toUpperCase()}${sentence.slice(1)} ${layoutPages.length || parts.length > 1 ? 'aren’t' : 'isn’t'} saved.`;
  return [unsaved, saving, exporting].filter(Boolean).join(' ');
}

export function leavePrompt(leaving: Leaving, unsaved: string | null, exportOnly = false): LeavePrompt | null {
  if (!unsaved) return null;
  if (exportOnly) {
    return leaving === 'update'
      ? { title: 'Cancel video export and restart?', detail: unsaved, discard: 'Cancel export and restart', keep: 'Keep exporting', hideSave: true }
      : { title: 'Cancel video export and close?', detail: unsaved, discard: 'Cancel export and close', keep: 'Keep exporting', hideSave: true };
  }
  return leaving === 'update'
    ? { title: 'Restart to update without saving?', detail: unsaved, discard: 'Discard and restart' }
    : { title: 'Close BattlePlan without saving?', detail: unsaved, discard: 'Discard and close' };
}
