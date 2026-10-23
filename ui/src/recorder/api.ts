// Commands and events of the Battle Plan Recorder tray app.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';
import type { CaptureMode, NextLoad, RecorderSettings, Recordings, Snapshot } from './status';

export interface SettingsPatch {
  recordEveryCampaign?: boolean;
  autoSave?: boolean;
  startWithWindows?: boolean;
  startWithReplay?: boolean;
  gameScore?: boolean;
  operationOutcomes?: boolean;
  politics?: boolean;
  captureMode?: CaptureMode;
}

export const recorderStatus = () => invoke<Snapshot>('recorder_status');
/** Tells the recorder the panel has drawn; it answers whether to open on the Stop prompt, and whether that prompt is all it shows. */
export const panelOpened = () => invoke<{ askStop: boolean; compact: boolean }>('panel_opened');
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
export const recordings = () => invoke<Recordings>('recordings');
/** Save or Discard a recording waiting to be saved. */
export const actOnWaiting = (session: string, campaign: string, discard: boolean) => invoke<void>('act_on_waiting', { session, campaign, discard });
/** Show a saved recording selected in its folder. */
export const revealRecording = (path: string) => invoke<void>('reveal_recording', { path });

export const onStatus = (handler: (snapshot: Snapshot) => void): Promise<UnlistenFn> =>
  listen<Snapshot>('recorder-status', event => handler(event.payload));
export const onAskStop = (handler: () => void): Promise<UnlistenFn> => listen('recorder-ask-stop', () => handler());
/** The recorder may consume a one-shot setting without a panel save. */
export const onSettings = (handler: () => void): Promise<UnlistenFn> => listen('recorder-settings', () => handler());
/** A recording was saved or discarded, or began or stopped waiting. */
export const onRecordings = (handler: () => void): Promise<UnlistenFn> => listen('recorder-recordings', () => handler());
