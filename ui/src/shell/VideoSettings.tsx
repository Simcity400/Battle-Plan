import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { desktopAvailable, errorMessage } from './api';
import { Icon } from '../components/Icons';

interface VideoOptions { useGameSettings: boolean; uiScale: number; vsync: boolean; mapUiScale: number | null }
interface VideoReply { settings: VideoOptions; gameSettings: VideoOptions | null; gameSettingsError: string | null; gameMapUiScale: number | null; mapUiScaleChoices: number[] }

const VSYNC = 'VSync';
const INTERFACE_SCALE = 'Interface scale';
const MAP_UI_SCALE = 'Map UI scale';
/** Option row labels, in order; settings search reveals rows by these. */
export const VIDEO_OPTION_LABELS = [VSYNC, INTERFACE_SCALE, MAP_UI_SCALE];
const MATCH_GAME = 'match';

export function useVideoSettings(enabled: boolean, userFolder?: string) {
  const [reply, setReply] = useState<VideoReply | null>(null);
  const [draft, setDraft] = useState<VideoOptions | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!enabled || !desktopAvailable) return;
    let active = true;
    setError(null); setReply(null); setDraft(null);
    void invoke<VideoReply>('video_settings').then(value => {
      if (active) { setReply(value); setDraft(value.settings); }
    }).catch(cause => { if (active) setError(errorMessage(cause)); });
    return () => { active = false; };
  }, [enabled, retry, userFolder]);
  return {
    draft, error, gameError: reply?.gameSettingsError, gameMapUiScale: reply?.gameMapUiScale ?? null, mapUiScaleChoices: reply?.mapUiScaleChoices ?? [],
    dirty: !!draft && JSON.stringify(draft) !== JSON.stringify(reply?.settings),
    canReset: !!reply?.gameSettings,
    update: (value: Partial<VideoOptions>) => { if (draft) setDraft({ ...draft, ...value, useGameSettings: false }); },
    reset: () => { if (reply?.gameSettings) setDraft(reply.gameSettings); },
    cancel: () => setDraft(reply?.settings ?? null),
    retry: () => setRetry(value => value + 1),
    save: async () => {
      if (!draft) return;
      const value = await invoke<VideoReply>('set_video_settings', { settings: draft });
      setReply(value); setDraft(value.settings);
    },
  };
}

export default function VideoControls({ video, busy }: { video: ReturnType<typeof useVideoSettings>; busy: boolean }) {
  const enabled = !!video.draft && !busy;
  return <>
    <label className="map-option-row"><span>{VSYNC} <span className="folder-info" tabIndex={0} role="img" aria-label="About VSync" data-tooltip="Synchronizes all native maps to the display to reduce tearing. This is a global graphics setting; browser preview uses the browser's presentation settings."><Icon name="info" /></span></span>
      {desktopAvailable ? <input type="checkbox" role="switch" disabled={!enabled} aria-checked={video.draft ? video.draft.vsync : 'mixed'} checked={video.draft?.vsync ?? false} onChange={event => video.update({ vsync: event.currentTarget.checked })} /> : <span className="graphics-unavailable">Desktop only</span>}
    </label>
    <label className="map-option-row"><span>{INTERFACE_SCALE} <span className="folder-info" tabIndex={0} role="img" aria-label="About interface scale" data-tooltip="Changes the size of app text, menus and buttons. Map widget sizes still use the dashboard Edit handles. In browser preview, use browser zoom."><Icon name="info" /></span></span>
      {desktopAvailable ? <select disabled={!enabled} value={video.draft?.uiScale ?? ''} onChange={event => video.update({ uiScale: Number(event.currentTarget.value) })}>
        {!video.draft && <option value="">{video.error ? 'Unavailable' : 'Loading...'}</option>}{[...new Set([...Array.from({ length: 16 }, (_, index) => 75 + index * 5), ...(video.draft ? [video.draft.uiScale] : [])])].sort((a, b) => a - b).map(scale => <option key={scale} value={scale}>{scale}%</option>)}
      </select> : <span className="graphics-unavailable">Browser zoom</span>}
    </label>
    <label className="map-option-row"><span>{MAP_UI_SCALE} <span className="folder-info" tabIndex={0} role="img" aria-label="About map UI scale" data-tooltip="Size of counters, map icons, labels, combat bubbles, the combat window and map tooltips. Match game draws them as large as the game shows them on this desktop, from its settings.txt and pdx_settings.txt display settings."><Icon name="info" /></span></span>
      {desktopAvailable ? <select disabled={!enabled} value={video.draft ? video.draft.mapUiScale ?? MATCH_GAME : ''} onChange={event => { const value = event.currentTarget.value; video.update({ mapUiScale: value === MATCH_GAME ? null : Number(value) }); }}>
        {!video.draft && <option value="">{video.error ? 'Unavailable' : 'Loading...'}</option>}
        <option value={MATCH_GAME}>{video.gameMapUiScale === null ? 'Match game (default)' : `Match game (${video.gameMapUiScale}%)`}</option>
        {[...new Set([...video.mapUiScaleChoices, ...(video.draft?.mapUiScale != null ? [video.draft.mapUiScale] : [])])].sort((a, b) => a - b).map(scale => <option key={scale} value={scale}>{scale}%</option>)}
      </select> : <span className="graphics-unavailable">Desktop only</span>}
    </label>
  </>;
}
