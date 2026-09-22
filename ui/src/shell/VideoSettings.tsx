import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { desktopAvailable, errorMessage } from './api';
import { Icon } from '../components/Icons';

interface VideoOptions { useGameSettings: boolean; uiScale: number; vsync: boolean }
interface VideoReply { settings: VideoOptions; gameSettings: VideoOptions | null; gameSettingsError: string | null }

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
    draft, error, gameError: reply?.gameSettingsError,
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
    <label className="map-option-row"><span>VSync <span className="folder-info" tabIndex={0} role="img" aria-label="About VSync" data-tooltip="Synchronizes all native maps to the display to reduce tearing. This is a global graphics setting; browser preview uses the browser's presentation settings."><Icon name="info" /></span></span>
      {desktopAvailable ? <input type="checkbox" role="switch" disabled={!enabled} aria-checked={video.draft ? video.draft.vsync : 'mixed'} checked={video.draft?.vsync ?? false} onChange={event => video.update({ vsync: event.currentTarget.checked })} /> : <span className="graphics-unavailable">Desktop only</span>}
    </label>
    <label className="map-option-row"><span>Interface scale <span className="folder-info" tabIndex={0} role="img" aria-label="About interface scale" data-tooltip="Changes the size of app text, menus and buttons. Map widget sizes still use the dashboard Edit handles. In browser preview, use browser zoom."><Icon name="info" /></span></span>
      {desktopAvailable ? <select disabled={!enabled} value={video.draft?.uiScale ?? ''} onChange={event => video.update({ uiScale: Number(event.currentTarget.value) })}>
        {!video.draft && <option value="">{video.error ? 'Unavailable' : 'Loading...'}</option>}{[...new Set([...Array.from({ length: 16 }, (_, index) => 75 + index * 5), ...(video.draft ? [video.draft.uiScale] : [])])].sort((a, b) => a - b).map(scale => <option key={scale} value={scale}>{scale}%</option>)}
      </select> : <span className="graphics-unavailable">Browser zoom</span>}
    </label>
  </>;
}
