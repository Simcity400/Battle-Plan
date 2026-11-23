export type Leaving = 'close' | 'update';

export const RESTARTING = 'Restarting…';

export function leavingTitle(leaving: Leaving): string {
  return leaving === 'update' ? 'Restarting BattlePlan to update…' : 'Closing BattlePlan…';
}

export interface LeavePrompt { title: string; detail: string; discard: string }

const listed = (names: readonly string[]) => names.length < 2 ? names.join('') : `${names.slice(0, -1).join(', ')} and ${names.at(-1)}`;

export function unsavedDetail(layoutPages: readonly string[], theme: string | null, writes: 'saving' | 'failed' | null = null): string | null {
  const parts = [layoutPages.length ? `Layout changes on ${listed(layoutPages)}` : null, theme === null ? null : theme ? `the theme “${theme}”` : 'your new theme'].filter(Boolean) as string[];
  const saving = writes === 'saving' ? 'A settings change is still being saved.' : writes === 'failed' ? 'A settings change isn’t saved.' : null;
  if (!parts.length) return saving;
  const sentence = parts.join(' and ');
  const unsaved = `${sentence[0].toUpperCase()}${sentence.slice(1)} ${layoutPages.length || parts.length > 1 ? 'aren’t' : 'isn’t'} saved.`;
  return saving ? `${unsaved} ${saving}` : unsaved;
}

export function leavePrompt(leaving: Leaving, unsaved: string | null): LeavePrompt | null {
  if (!unsaved) return null;
  return leaving === 'update'
    ? { title: 'Restart to update without saving?', detail: unsaved, discard: 'Discard and restart' }
    : { title: 'Close BattlePlan without saving?', detail: unsaved, discard: 'Discard and close' };
}
