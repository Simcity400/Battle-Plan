import { invoke } from '@tauri-apps/api/core';
import { invokeWrite } from '../shell/writes';
import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';
import type { CaptureMode, NextLoad, PanelOpening, RecorderSettings, Recordings, RecordingsView, Snapshot } from './status';
import type { Compression } from './compression';

export interface SettingsPatch {
  compression?: Compression;
  autoCompress?: boolean;
  recordEveryCampaign?: boolean;
  autoSave?: boolean;
  startWithWindows?: boolean;
  startWithReplay?: boolean;
  captureMode?: CaptureMode;
}

export const recorderStatus = () => invoke<Snapshot>('recorder_status');
export const panelOpened = (panel: string) => invoke<PanelOpening>('panel_opened', { panel });
export const panelFit = (height: number) => invoke<void>('panel_fit', { height });
export const closePanel = (revision: number) => invoke<boolean>('close_panel', { revision });
export const promptSeen = (prompt: number) => invoke<void>('prompt_seen', { prompt });
export const startRecording = () => invoke<void>('start_recording');
export const stopRecording = (discard: boolean, sessionKey: number) => invoke<void>('stop_recording', { discard, sessionKey });
export const cancelStopPrompt = (prompt: number) => invoke<void>('cancel_stop_prompt', { prompt });
export const recorderSettings = () => invoke<RecorderSettings>('recorder_settings');
export const updateSettings = (patch: SettingsPatch) => invokeWrite<RecorderSettings>('update_recorder_settings', { patch });
export const setCampaignChoice = (campaign: string, choice: { record?: boolean; next?: NextLoad }) =>
  invokeWrite<RecorderSettings>('set_campaign_choice', { campaign, record: choice.record ?? null, next: choice.next ?? null });
export const chooseSaveFolder = () => invokeWrite<RecorderSettings | null>('choose_save_folder');
export const setUserFolder = (choose: boolean) => invokeWrite<RecorderSettings | null>('set_user_folder', { choose });
export const openRecordingsFolder = () => invoke<void>('open_recordings_folder');
export const showSessionFiles = () => invoke<void>('show_session_files');
export const openReplay = (path: string | null) => invoke<void>('open_replay', { path });
export const recordings = (view: RecordingsView) => invoke<Recordings>('recordings', { ...view });
export const actOnWaiting = (session: string, campaign: string, discard: boolean) => invoke<string | null>('act_on_waiting', { session, campaign, discard });
export const revealRecording = (path: string) => invoke<void>('reveal_recording', { path });
export const compressRecording = (path: string) => invoke<void>('compress_recording', { path });

export const onStatus = (handler: (snapshot: Snapshot) => void): Promise<UnlistenFn> =>
  listen<Snapshot>('recorder-status', event => handler(event.payload));
export const onSettings = (handler: () => void): Promise<UnlistenFn> => listen('recorder-settings', () => handler());
export const onRecordings = (handler: () => void): Promise<UnlistenFn> => listen('recorder-recordings', () => handler());
