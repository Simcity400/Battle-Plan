import { useEffect, useState } from 'react';
import { Select } from '../components/Select';
import { invoke } from '@tauri-apps/api/core';
import { desktopAvailable, errorMessage } from './api';
import { Icon } from '../components/Icons';
import { sameDisplay } from './videoOptions';
import type { VideoOptions } from './videoOptions';
interface VideoReply { settings: VideoOptions; gameSettings: VideoOptions | null; gameSettingsError: string | null; gameMapUiScale: number | null; mapUiScaleChoices: number[] }

const VSYNC = 'VSync';
const FRAME_LIMIT = 'Frame rate limit';
const INTERFACE_SCALE = 'Interface scale';
const MAP_UI_SCALE = 'Map UI scale';
/** Option row labels, in order; settings search reveals rows by these. */
export const VIDEO_OPTION_LABELS = [VSYNC, FRAME_LIMIT, INTERFACE_SCALE, MAP_UI_SCALE];
const MATCH_GAME = 'match';
/** The frame rate limits the maps accept (`MAP_FRAME_LIMIT_RANGE`). */
const MIN_FRAME_LIMIT = 1;
const MAX_FRAME_LIMIT = 1000;

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
    dirty: !!draft && !sameDisplay(draft, reply?.settings),
    canReset: !!reply?.gameSettings,
    /** The display settings already are the game's own, so reverting would change nothing. */
    atGame: !!reply?.gameSettings && sameDisplay(draft, reply.gameSettings),
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
    <div className="map-option-row"><span>{FRAME_LIMIT} <span className="folder-info" tabIndex={0} role="img" aria-label="About the frame rate limit" data-tooltip="Holds every native map to at most this many evenly spaced frames a second, which a weaker machine can draw steadily and a laptop on battery pays less for. Empty draws as the display and VSync allow."><Icon name="info" /></span></span>
      {desktopAvailable ? <span className="frame-limit-field"><input type="number" min={MIN_FRAME_LIMIT} max={MAX_FRAME_LIMIT} step={1} placeholder="Off" aria-label={FRAME_LIMIT} disabled={!enabled || !video.draft}
        value={video.draft?.mapFrameLimit ?? ''}
        onChange={event => {
          // Cleared means no limit, as the setting starts.
          const hz = event.currentTarget.value === '' ? null : Math.round(Number(event.currentTarget.value));
          if (hz === null || (hz >= MIN_FRAME_LIMIT && hz <= MAX_FRAME_LIMIT)) video.update({ mapFrameLimit: hz });
        }} /><small>frames a second</small></span>
        : <span className="graphics-unavailable">Desktop only</span>}
    </div>
    <div className="map-option-row"><span>{INTERFACE_SCALE} <span className="folder-info" tabIndex={0} role="img" aria-label="About interface scale" data-tooltip="Changes the size of app text, menus and buttons. Map widget sizes still use the dashboard Edit handles. In browser preview, use browser zoom."><Icon name="info" /></span></span>
      {desktopAvailable ? <Select label={INTERFACE_SCALE} disabled={!enabled} value={String(video.draft?.uiScale ?? '')} onChange={value => video.update({ uiScale: Number(value) })}
        options={[...(video.draft ? [] : [{ value: '', label: video.error ? 'Unavailable' : 'Loading...' }]),
          ...[...new Set([...Array.from({ length: 16 }, (_, index) => 75 + index * 5), ...(video.draft ? [video.draft.uiScale] : [])])].sort((a, b) => a - b).map(scale => ({ value: String(scale), label: `${scale}%` }))]} />
        : <span className="graphics-unavailable">Browser zoom</span>}
    </div>
    <div className="map-option-row"><span>{MAP_UI_SCALE} <span className="folder-info" tabIndex={0} role="img" aria-label="About map UI scale" data-tooltip="Size of counters, map icons, labels, combat bubbles, the combat window and map tooltips. Match game draws them as large as the game shows them on this desktop, from its settings.txt and pdx_settings.txt display settings."><Icon name="info" /></span></span>
      {desktopAvailable ? <Select label={MAP_UI_SCALE} disabled={!enabled} value={video.draft ? String(video.draft.mapUiScale ?? MATCH_GAME) : ''}
        onChange={value => video.update({ mapUiScale: value === MATCH_GAME ? null : Number(value) })}
        options={[...(video.draft ? [] : [{ value: '', label: video.error ? 'Unavailable' : 'Loading...' }]),
          { value: MATCH_GAME, label: video.gameMapUiScale === null ? 'Match game (default)' : `Match game (${video.gameMapUiScale}%)` },
          ...[...new Set([...video.mapUiScaleChoices, ...(video.draft?.mapUiScale != null ? [video.draft.mapUiScale] : [])])].sort((a, b) => a - b).map(scale => ({ value: String(scale), label: `${scale}%` }))]} />
        : <span className="graphics-unavailable">Desktop only</span>}
    </div>
  </>;
}
