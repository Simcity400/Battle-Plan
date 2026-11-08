// What the update button and settings show for a state, as T3 Code's update control does.
import type { UpdateChannel, UpdateState } from './api';

export type UpdateAction = 'check' | 'download' | 'install' | 'none';

/** What clicking the update button does: the offer decides, not the last error. */
export function updateAction(state: UpdateState | null): UpdateAction {
  if (!state?.enabled || state.status === 'checking' || state.status === 'downloading') return 'none';
  if (state.downloadedVersion) return 'install';
  if (state.availableVersion) return 'download';
  return 'check';
}

/** The button waits while an action runs, and while the recorder may not be replaced. */
export function updateDisabled(state: UpdateState | null): boolean {
  return updateAction(state) === 'none' || (updateAction(state) === 'install' && state?.installBlocked != null);
}

const percent = (state: UpdateState) => typeof state.downloadPercent === 'number' ? ` (${Math.floor(state.downloadPercent)}%)` : '';

/** The button's tooltip: what it shows and what a click does. */
export function updateTooltip(state: UpdateState | null): string {
  if (!state) return 'Checking for updates…';
  if (!state.enabled) return state.message ?? 'Updates are off in this build.';
  if (state.status === 'checking') return 'Checking for updates…';
  if (state.status === 'downloading') return `Downloading update ${state.availableVersion ?? ''}${percent(state)}`.replace('  ', ' ');
  if (state.downloadedVersion) {
    if (state.installBlocked) return `Update ${state.downloadedVersion} downloaded. ${state.installBlocked}`;
    if (state.errorContext === 'install') return `Install failed for ${state.downloadedVersion}: ${state.message}. Click to retry.`;
    return `Update ${state.downloadedVersion} downloaded. Click to restart and install.`;
  }
  if (state.availableVersion) {
    if (state.errorContext === 'download') return `${state.message} Click to retry.`;
    return `Update ${state.availableVersion} ready to download`;
  }
  if (state.status === 'error') return `${state.message ?? 'The update check failed.'} Click to check again.`;
  if (state.status === 'up-to-date') return `Up to date · ${state.currentVersion}`;
  return 'Check for updates';
}

/** The settings button's label for the same action. */
export function updateButtonLabel(state: UpdateState | null): string {
  if (state?.status === 'checking') return 'Checking…';
  if (state?.status === 'downloading') return `Downloading…${state.downloadPercent == null ? '' : ` ${Math.floor(state.downloadPercent)}%`}`;
  const action = updateAction(state);
  if (action === 'install') return 'Restart to update';
  if (action === 'download') return 'Download';
  return state?.status === 'up-to-date' ? 'Check again' : 'Check for updates';
}

/** The settings row's line under the version. */
export function updateDescription(state: UpdateState | null): string {
  if (!state) return 'Reading the update state…';
  if (!state.enabled) return state.message ?? 'Updates are off in this build.';
  if (state.installBlocked && state.downloadedVersion) return `Update ${state.downloadedVersion} is downloaded. ${state.installBlocked}`;
  if (state.message) return state.message;
  if (state.downloadedVersion) return `Update ${state.downloadedVersion} is downloaded. Restart to install it in both apps.`;
  if (state.availableVersion) return `Update ${state.availableVersion} is available.`;
  if (state.status === 'up-to-date') return `Up to date${state.checkedAt ? `, checked ${new Date(state.checkedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` : ''}.`;
  return 'Battle Plan and Battle Plan Recorder update together.';
}

export const CHANNEL_LABELS: Record<UpdateChannel, string> = { stable: 'Stable', nightly: 'Nightly' };

/** What the channel row says: the choice, and when a nightly build stays put on Stable. */
export function channelDescription(state: UpdateState | null): string {
  if (!state) return '';
  if (!state.enabled) return 'This build does not update, so its channel has no effect.';
  if (state.channel === 'stable' && state.build === 'nightly')
    return `This nightly build stays until a stable release newer than ${state.currentVersion} comes out.`;
  return state.channel === 'nightly'
    ? 'A build of the latest changes, made each day they change. For both apps.'
    : 'Tested releases. For both apps.';
}

export function confirmInstall(version: string): string {
  return `Battle Plan and Battle Plan Recorder close, update to ${version} and reopen.`;
}

/** At most `shown` notes, and how many were left out. */
export function shownNotes(notes: readonly string[], shown = 6): { items: string[]; more: number } {
  return { items: notes.slice(0, shown), more: Math.max(0, notes.length - shown) };
}
