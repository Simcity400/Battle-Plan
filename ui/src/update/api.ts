import { invoke, isTauri } from '@tauri-apps/api/core';
import { invokeWrite } from '../shell/writes';
import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { nextInstallAttempt } from '../shell/installAttempt';

export type UpdateStatus = 'disabled' | 'idle' | 'checking' | 'up-to-date' | 'available' | 'downloading' | 'downloaded' | 'error';
export type UpdateChannel = 'stable' | 'nightly';
export type UpdateBuild = 'stable' | 'nightly' | 'preview' | 'development';

export interface UpdateState {
  enabled: boolean;
  status: UpdateStatus;
  channel: UpdateChannel;
  build: UpdateBuild;
  currentVersion: string;
  availableVersion: string | null;
  downloadedVersion: string | null;
  releasedAt: string | null;
  notes: string[];
  downloadPercent: number | null;
  checkedAt: number | null;
  message: string | null;
  errorContext: 'check' | 'download' | 'install' | null;
  installBlocked: string | null;
  channelWarning: string | null;
  downgrade: boolean;
}

export const updatesAvailable = isTauri();

export const updateState = () => invoke<UpdateState>('app_update_state');
export const checkForUpdate = () => invoke<UpdateState>('app_update_check');
export const downloadUpdate = () => invoke<UpdateState>('app_update_download');
export const installUpdate = () => invoke<void>('app_update_install', nextInstallAttempt());
export const setUpdateChannel = (channel: UpdateChannel) => invokeWrite<UpdateState>('app_update_set_channel', { channel });
export const openRelease = (version: string) => invoke<void>('app_update_open_release', { version });
export const onUpdateState = (handler: (state: UpdateState) => void): Promise<UnlistenFn> =>
  listen<UpdateState>('app-update-state', event => handler(event.payload));
