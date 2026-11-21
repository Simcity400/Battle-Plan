export type Leaving = 'close' | 'update';

export interface LeavePrompt { title: string; detail: string; discard: string; failed: string }

const listed = (names: readonly string[]) => names.length < 2 ? names.join('') : `${names.slice(0, -1).join(', ')} and ${names.at(-1)}`;

export function unsavedDetail(layoutPages: readonly string[], theme: string | null): string | null {
  const parts = [layoutPages.length ? `Layout changes on ${listed(layoutPages)}` : null, theme === null ? null : theme ? `the theme “${theme}”` : 'your new theme'].filter(Boolean) as string[];
  if (!parts.length) return null;
  const sentence = parts.join(' and ');
  return `${sentence[0].toUpperCase()}${sentence.slice(1)} ${layoutPages.length || parts.length > 1 ? 'aren’t' : 'isn’t'} saved.`;
}

export function leavePrompt(leaving: Leaving, unsaved: string | null): LeavePrompt | null {
  if (!unsaved) return null;
  return leaving === 'update'
    ? { title: 'Restart to update without saving?', detail: unsaved, discard: 'Discard and restart', failed: 'Couldn’t restart to update.' }
    : { title: 'Close BattlePlan without saving?', detail: unsaved, discard: 'Discard and close', failed: 'Couldn’t close BattlePlan.' };
}
