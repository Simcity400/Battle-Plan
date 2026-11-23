import { useEffect, useId, useRef, useState } from 'react';
import { Select } from '../components/Select';
import { invoke } from '@tauri-apps/api/core';
import { invokeWrite } from './writes';
import { desktopAvailable, errorMessage } from './api';
import { Icon } from '../components/Icons';
import { FailureLine } from '../components/FailureLine';
import { FRAME_LIMIT_HINT, MAX_FRAME_LIMIT, MIN_FRAME_LIMIT, frameLimitInput, sameDisplay } from './videoOptions';
import type { VideoOptions } from './videoOptions';
import { writeSlot } from './pendingWrites';
interface VideoReply { settings: VideoOptions; gameSettings: VideoOptions | null; gameSettingsError: string | null; gameMapUiScale: number | null; mapUiScaleChoices: number[]; warning: string | null }

const VSYNC = 'VSync';
const FRAME_LIMIT = 'Frame rate limit';
const INTERFACE_SCALE = 'Interface scale';
const MAP_UI_SCALE = 'Map UI scale';
export const VIDEO_OPTION_LABELS = [VSYNC, FRAME_LIMIT, INTERFACE_SCALE, MAP_UI_SCALE];
const MATCH_GAME = 'match';

export function useVideoSettings(enabled: boolean, userFolder?: string) {
  const [reply, setReply] = useState<VideoReply | null>(null);
  const [draft, setDraft] = useState<VideoOptions | null>(null);
  const [error, setError] = useState<{ detail: string; write: VideoOptions | null } | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [pending, setPending] = useState(false);
  const writes = useRef({ next: null as VideoOptions | null, running: false, live: true, slot: writeSlot() });
  useEffect(() => { const queue = writes.current; queue.live = true; return () => { queue.live = false; if (!queue.running) queue.slot.discard(); }; }, []);
  useEffect(() => {
    if (!enabled || !desktopAvailable) return;
    let active = true;
    setError(null); setReply(null); setDraft(null);
    void invoke<VideoReply>('video_settings').then(value => {
      if (active) { setReply(value); setDraft(value.settings); }
    }).catch(cause => { if (active) setError({ detail: errorMessage(cause), write: null }); }).finally(() => { if (active) setPending(false); });
    return () => { active = false; };
  }, [enabled, retry, userFolder]);
  const write = async () => {
    const queue = writes.current;
    if (queue.running) return;
    queue.running = true;
    queue.slot.enter();
    setPending(true);
    let failed: VideoOptions | null = null;
    while (queue.next) {
      const settings = queue.next; queue.next = null;
      try {
        const value = await invokeWrite<VideoReply>('set_video_settings', { settings });
        failed = null;
        if (queue.live) { setReply(value); setError(null); setWarning(value.warning); if (!queue.next) setDraft(value.settings); }
      } catch (cause) {
        failed = settings;
        if (queue.live) setError({ detail: errorMessage(cause), write: settings });
      }
    }
    queue.running = false;
    const again = failed;
    if (!again) queue.slot.saved(); else if (queue.live) queue.slot.failed(() => { queue.next ??= again; void write(); }); else queue.slot.discard();
    if (queue.live) setPending(false);
  };
  const apply = (next: VideoOptions) => {
    if (sameDisplay(next, draft ?? undefined)) return;
    setDraft(next); writes.current.slot.enter(); writes.current.next = next; void write();
  };
  return {
    draft, error: error?.detail ?? null, warning, writeFailed: !!error?.write, pending, gameError: reply?.gameSettingsError, gameMapUiScale: reply?.gameMapUiScale ?? null, mapUiScaleChoices: reply?.mapUiScaleChoices ?? [],
    canReset: !!reply?.gameSettings,
    atGame: !!reply?.gameSettings && sameDisplay(draft, reply.gameSettings),
    update: (value: Partial<VideoOptions>) => { if (draft) apply({ ...draft, ...value, useGameSettings: false }); },
    reset: () => { if (reply?.gameSettings) apply(reply.gameSettings); },
    retry: () => {
      if (!error?.write) { setPending(true); setRetry(value => value + 1); return; }
      writes.current.next ??= error.write; void write();
    },
  };
}

function FrameLimit({ value, disabled, onCommit }: { value: number | null; disabled: boolean; onCommit: (value: number | null) => void }) {
  const shown = value === null ? '' : String(value);
  const hintId = useId();
  const [text, setText] = useState(shown);
  const [bad, setBad] = useState(false);
  const [invalid, setInvalid] = useState(false);
  useEffect(() => { setText(shown); setBad(false); setInvalid(false); }, [shown]);
  const commit = () => {
    const parsed = frameLimitInput(text, bad);
    setInvalid(!parsed);
    if (parsed && parsed.value !== value) onCommit(parsed.value);
  };
  return <>
    <span className="frame-limit-field"><input type="number" min={MIN_FRAME_LIMIT} max={MAX_FRAME_LIMIT} step={1} placeholder="Off" aria-label={FRAME_LIMIT} disabled={disabled}
      aria-invalid={invalid || undefined} aria-describedby={invalid ? hintId : undefined} value={text}
      onChange={event => { setText(event.currentTarget.value); setBad(event.currentTarget.validity.badInput); setInvalid(false); }} onBlur={commit}
      onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); commit(); } }} /><small>frames a second</small></span>
    {invalid && <div className="map-option-note" id={hintId}><FailureLine message={FRAME_LIMIT_HINT} /></div>}
  </>;
}

const unavailable = <span className="settings-value">Desktop only</span>;

export default function VideoControls({ video, busy }: { video: ReturnType<typeof useVideoSettings>; busy: boolean }) {
  const enabled = !!video.draft && !busy;
  const loading = video.error ? 'Unavailable' : 'Loading…';
  return <>
    <label className="map-option-row"><span>{VSYNC} <span className="folder-info" tabIndex={0} role="img" aria-label="About VSync" onClick={event => event.preventDefault()} data-tooltip="Synchronises every map to the display to reduce tearing. In the browser, the browser’s own presentation settings apply."><Icon name="info" /></span></span>
      {!desktopAvailable ? unavailable : video.draft ? <input type="checkbox" role="switch" disabled={!enabled} checked={video.draft.vsync} onChange={event => video.update({ vsync: event.currentTarget.checked })} />
        : <span className="map-option-unknown" role="img" aria-label={`${VSYNC}: ${loading}`}>—</span>}
    </label>
    <div className="map-option-row"><span>{FRAME_LIMIT} <span className="folder-info" tabIndex={0} role="img" aria-label="About the frame rate limit" data-tooltip="Holds every map to at most this many evenly spaced frames a second, which a weaker machine can draw steadily and a laptop on battery pays less for. Empty draws as fast as the display and VSync allow."><Icon name="info" /></span></span>
      {!desktopAvailable ? unavailable : video.draft ? <FrameLimit value={video.draft.mapFrameLimit} disabled={!enabled} onCommit={hz => video.update({ mapFrameLimit: hz })} />
        : <span className="map-option-unknown" role="img" aria-label={`${FRAME_LIMIT}: ${loading}`}>—</span>}
    </div>
    <div className="map-option-row"><span>{INTERFACE_SCALE} <span className="folder-info" tabIndex={0} role="img" aria-label="About interface scale" data-tooltip="Changes the size of app text, menus and buttons. Map widget sizes still use the dashboard Edit handles. In the browser, use browser zoom."><Icon name="info" /></span></span>
      {desktopAvailable ? <Select label={INTERFACE_SCALE} disabled={!enabled} value={String(video.draft?.uiScale ?? '')} onChange={value => video.update({ uiScale: Number(value) })}
        options={[...(video.draft ? [] : [{ value: '', label: loading }]),
          ...[...new Set([...Array.from({ length: 16 }, (_, index) => 75 + index * 5), ...(video.draft ? [video.draft.uiScale] : [])])].sort((a, b) => a - b).map(scale => ({ value: String(scale), label: `${scale}%` }))]} />
        : unavailable}
    </div>
    <div className="map-option-row"><span>{MAP_UI_SCALE} <span className="folder-info" tabIndex={0} role="img" aria-label="About map UI scale" data-tooltip="Size of counters, map icons, labels, combat bubbles, the combat window and map tooltips. Match game draws them as large as the game shows them on this desktop, from its own display settings."><Icon name="info" /></span></span>
      {desktopAvailable ? <Select label={MAP_UI_SCALE} disabled={!enabled} value={video.draft ? String(video.draft.mapUiScale ?? MATCH_GAME) : ''}
        onChange={value => video.update({ mapUiScale: value === MATCH_GAME ? null : Number(value) })}
        options={[...(video.draft ? [] : [{ value: '', label: loading }]),
          { value: MATCH_GAME, label: video.gameMapUiScale === null ? 'Match game' : `Match game (${video.gameMapUiScale}%)` },
          ...[...new Set([...video.mapUiScaleChoices, ...(video.draft?.mapUiScale != null ? [video.draft.mapUiScale] : [])])].sort((a, b) => a - b).map(scale => ({ value: String(scale), label: `${scale}%` }))]} />
        : unavailable}
    </div>
  </>;
}
