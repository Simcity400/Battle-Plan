import { plainFailure } from '../dashboard/queryError';
import type { UpdateChannel, UpdateState } from './api';

export type UpdateAction = 'check' | 'download' | 'install' | 'none';

export function updateAction(state: UpdateState | null): UpdateAction {
  if (!state?.enabled || state.status === 'checking' || state.status === 'downloading') return 'none';
  if (state.downloadedVersion) return 'install';
  if (state.availableVersion) return 'download';
  return 'check';
}

export function updateDisabled(state: UpdateState | null): boolean {
  return updateAction(state) === 'none' || (updateAction(state) === 'install' && state?.installBlocked != null);
}

export interface UpdateFailure { text: string; detail: string | null }

export const UPDATE_READ_FAILED = 'Couldn’t check for updates.';

export function updateFailure(state: UpdateState | null): UpdateFailure | null {
  if (!state?.enabled || state.status === 'checking' || state.status === 'downloading') return null;
  const detail = state.message;
  if (state.downloadedVersion) return state.errorContext === 'install' ? { text: `Couldn’t install update ${state.downloadedVersion}.`, detail } : null;
  if (state.availableVersion) return state.errorContext === 'download' ? { text: `Couldn’t download update ${state.availableVersion}.`, detail } : null;
  if (state.errorContext || state.status === 'error') return { text: UPDATE_READ_FAILED, detail };
  return detail ? { text: 'Couldn’t read the update channel.', detail } : null;
}

export function blockedReason(state: UpdateState | null): string | null {
  return state?.installBlocked ? plainFailure(state.installBlocked).text : null;
}

const percent = (state: UpdateState) => typeof state.downloadPercent === 'number' ? ` (${Math.floor(state.downloadPercent)}%)` : '';

const offer = (state: UpdateState) => state.downgrade ? 'version' : 'update';
const Offer = (state: UpdateState) => state.downgrade ? 'Version' : 'Update';

export function restartLabel(state: UpdateState | null): string {
  return state?.downgrade ? 'Restart to switch' : 'Restart to update';
}

export function updateTooltip(state: UpdateState | null): string {
  if (!state) return 'Checking for updates…';
  if (!state.enabled) return state.message ? plainFailure(state.message).text : 'Updates are off in this build.';
  if (state.status === 'checking') return 'Checking for updates…';
  if (state.status === 'downloading') return `Downloading ${offer(state)} ${state.availableVersion ?? ''}${percent(state)}`.replace('  ', ' ');
  const failure = updateFailure(state);
  if (failure) return `${failure.text.replace(/\.$/, '')} · Try again`;
  if (state.downloadedVersion) {
    if (state.installBlocked) return `${Offer(state)} ${state.downloadedVersion} downloaded. ${blockedReason(state)}`;
    return `${Offer(state)} ${state.downloadedVersion} downloaded · ${restartLabel(state)}`;
  }
  if (state.availableVersion) return `${Offer(state)} ${state.availableVersion} ready to download`;
  if (state.status === 'up-to-date') return `Up to date · ${state.currentVersion}`;
  return 'Check for updates';
}

export function updateNews(state: UpdateState | null): boolean {
  const action = updateAction(state);
  return action === 'download' || action === 'install' || state?.status === 'downloading' || updateFailure(state) != null;
}

export function updateButtonLabel(state: UpdateState | null): string {
  if (state?.status === 'checking') return 'Checking…';
  if (state?.status === 'downloading') return `Downloading…${state.downloadPercent == null ? '' : ` ${Math.floor(state.downloadPercent)}%`}`;
  const action = updateAction(state);
  if (action === 'install') return restartLabel(state);
  if (action === 'download') return 'Download';
  return state?.status === 'up-to-date' ? 'Check again' : 'Check for updates';
}

export function updateDescription(state: UpdateState | null): string {
  if (!state) return 'Reading the update state…';
  if (!state.enabled) return state.message ? plainFailure(state.message).text : 'Updates are off in this build.';
  if (state.installBlocked && state.downloadedVersion) return `${Offer(state)} ${state.downloadedVersion} is downloaded. ${blockedReason(state)}`;
  if (state.downloadedVersion) return state.downgrade ? `Version ${state.downloadedVersion} is downloaded. Restart to switch both apps to it.`
    : `Update ${state.downloadedVersion} is downloaded. Restart to install it in both apps.`;
  if (state.availableVersion) return state.downgrade ? `Version ${state.availableVersion} is available on this channel. It’s older than ${state.currentVersion}.`
    : `Update ${state.availableVersion} is available.`;
  if (state.status === 'up-to-date') return `Up to date${state.checkedAt ? ` (checked ${new Date(state.checkedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })})` : ''}.`;
  return 'BattlePlan and BattlePlan Recorder update together.';
}

export function channelLocked(state: UpdateState | null, pending: boolean): boolean {
  return !state?.enabled || pending || state.status === 'checking' || state.status === 'downloading';
}

export const CHANNEL_LABELS: Record<UpdateChannel, string> = { stable: 'Stable', nightly: 'Nightly' };

export function channelDescription(state: UpdateState | null): string {
  if (!state) return '';
  if (state.channelWarning) return `Changed to ${CHANNEL_LABELS[state.channel]}, but Windows didn’t confirm the choice reached the disk.`;
  if (!state.enabled) return 'This build doesn’t update, so its channel has no effect.';
  return state.channel === 'nightly'
    ? 'Nightly: a build of each day’s changes for BattlePlan and the recorder.'
    : 'Stable: tested releases for BattlePlan and the recorder.';
}

export function restartQuestion(version: string, downgrade: boolean): string {
  return downgrade ? `Restart to switch to ${version}?` : `Restart to update to ${version}?`;
}

export function confirmInstall(version: string, downgrade: boolean): string {
  return downgrade
    ? `BattlePlan and BattlePlan Recorder close, switch to ${version}, an older version, and reopen. Recordings made with this build may not open in it.`
    : `BattlePlan and BattlePlan Recorder close, update to ${version} and reopen.`;
}

export function shownNotes(notes: readonly string[], shown = 6): { items: string[]; more: number } {
  return { items: notes.slice(0, shown), more: Math.max(0, notes.length - shown) };
}
