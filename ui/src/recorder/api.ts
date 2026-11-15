// Commands and events of the Battle Plan Recorder tray app.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';
import type { CaptureMode, NextLoad, RecorderSettings, Recordings, RecordingsView, Snapshot } from './status';
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
/** Why the Stop prompt was asked for: Quit waits on it, or Discard was chosen and only its confirmation is left. */
export interface Ask { quitting: boolean; discarding: boolean }
/** Tells the recorder the panel has drawn; it answers whether to open on the Stop prompt, whether that prompt is all it shows, and why. */
export const panelOpened = (panel: string) => invoke<{ askStop: boolean; compact: boolean } & Ask>('panel_opened', { panel });
export const panelFit = (height: number) => invoke<void>('panel_fit', { height });
export const closePanel = () => invoke<void>('close_panel');
export const startRecording = () => invoke<void>('start_recording');
export const stopRecording = (discard: boolean) => invoke<void>('stop_recording', { discard });
/** Stop pressed again: the prompt is cancelled, and with it a Quit that opened it. */
export const cancelStopPrompt = () => invoke<void>('cancel_stop_prompt');
export const recorderSettings = () => invoke<RecorderSettings>('recorder_settings');
export const updateSettings = (patch: SettingsPatch) => invoke<RecorderSettings>('update_recorder_settings', { patch });
export const setCampaignChoice = (campaign: string, choice: { record?: boolean; next?: NextLoad }) =>
  invoke<RecorderSettings>('set_campaign_choice', { campaign, record: choice.record ?? null, next: choice.next ?? null });
export const chooseSaveFolder = () => invoke<RecorderSettings | null>('choose_save_folder');
export const setUserFolder = (choose: boolean) => invoke<RecorderSettings | null>('set_user_folder', { choose });
export const openRecordingsFolder = () => invoke<void>('open_recordings_folder');
export const openDetails = () => invoke<void>('open_details');
export const openReplay = (path: string | null) => invoke<void>('open_replay', { path });
export const recordings = (view: RecordingsView) => invoke<Recordings>('recordings', { ...view });
/** Save or Discard a recording waiting to be saved. */
/** Resolves to why a save named its countries by tag, when it did. */
export const actOnWaiting = (session: string, campaign: string, discard: boolean) => invoke<string | null>('act_on_waiting', { session, campaign, discard });
/** Show a saved recording selected in its folder. */
export const revealRecording = (path: string) => invoke<void>('reveal_recording', { path });
/** Compress a saved recording to the smaller High size in the background, one at a time. */
export const compressRecording = (path: string) => invoke<void>('compress_recording', { path });

export const onStatus = (handler: (snapshot: Snapshot) => void): Promise<UnlistenFn> =>
  listen<Snapshot>('recorder-status', event => handler(event.payload));
/** Quit or Discard asks with the panel open. */
export const onAskStop = (handler: (ask: Ask) => void): Promise<UnlistenFn> => listen<Ask>('recorder-ask-stop', event => handler(event.payload));
/** The recorder may consume a one-shot setting without a panel save. */
export const onSettings = (handler: () => void): Promise<UnlistenFn> => listen('recorder-settings', () => handler());
/** A recording was saved or discarded, or began or stopped waiting. */
export const onRecordings = (handler: () => void): Promise<UnlistenFn> => listen('recorder-recordings', () => handler());
