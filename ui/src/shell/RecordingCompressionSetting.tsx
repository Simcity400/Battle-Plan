import { useEffect, useState } from 'react';
import { Select } from '../components/Select';
import type { CompactionProgress, DashboardClient, RecorderSettings, RecordingCompression } from '../dashboard/client';
import { errorMessage, recordingName } from './api';

export function savedSize(bytes: number): string {
  return bytes < 2 ** 20 ? `${Math.round(bytes / 1024)} KB` : bytes < 2 ** 30
    ? `${(bytes / 2 ** 20).toFixed(1)} MB` : `${(bytes / 2 ** 30).toFixed(1)} GB`;
}

export function compactionSummary(progress: CompactionProgress): string {
  const done = `${progress.completed} of ${progress.total} recordings`;
  const skipped = progress.skipped ? ` · ${progress.skipped} left unchanged` : '';
  return `${progress.running ? 'Compacting' : 'Checked'} ${done} · Saved ${savedSize(progress.bytesSaved)}${skipped}`;
}

/** Recorder preferences stay in native settings; the recorder compacts automatically, this app on request. */
export default function RecordingCompressionSetting({ client, folder }: { client: DashboardClient; folder: string | null }) {
  const [settings, setSettings] = useState<RecorderSettings | null>(null);
  const [progress, setProgress] = useState<CompactionProgress | null>(null);
  const [saving, setSaving] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    setSettings(null); setError(null);
    void client.recorderSettings!().then(value => { if (active) setSettings(value); }, cause => { if (active) setError(errorMessage(cause)); });
    return () => { active = false; };
  }, [client]);
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      let running = false;
      try {
        const value = await client.compactionProgress!();
        if (!active) return;
        running = value.running;
        setProgress(value);
      } catch (cause) { if (active) setError(errorMessage(cause)); }
      if (active) timer = setTimeout(() => { void poll(); }, running ? 500 : 2000);
    };
    void poll();
    return () => { active = false; clearTimeout(timer); };
  }, [client]);
  const save = async (compression: RecordingCompression) => {
    setSaving(true); setError(null);
    try { setSettings(await client.setRecordingCompression!(compression)); }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setSaving(false); }
  };
  const compactAll = async () => {
    setStarting(true); setError(null);
    try { setProgress(await client.compactReplayFolder!()); }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setStarting(false); }
  };
  return <>
    <div className="settings-row" id="folders-compression">
      <span className="settings-row-text"><strong>Recording compression</strong>
        <small>Recorder setting. With Small, Battle Plan Recorder compacts each recording after the game closes.</small></span>
      <Select label="Recording compression" value={settings?.compression ?? 'fast'} disabled={!settings || saving}
        options={[{ value: 'fast', label: 'Fast (larger files)' }, { value: 'small', label: 'Small (slower)' }]}
        onChange={value => { void save(value as RecordingCompression); }} />
    </div>
    <div className="settings-row" id="folders-compact">
      <span className="settings-row-text"><strong>Compact existing recordings</strong>
        <small>Compact finished recordings in your replay folder. You can keep campaigns open.</small></span>
      <button type="button" className="button small" disabled={!folder || starting || progress?.running}
        onClick={() => { void compactAll(); }}>Compact all</button>
    </div>
    {progress?.gameRunning && <div className="settings-group-body"><p className="settings-help">The game is running. Compacting may slow it briefly.</p></div>}
    {progress && (progress.running || progress.total > 0 || progress.unchanged.length > 0) && <div className="settings-group-body" role="status" aria-live="polite">
      <p className="settings-help">{compactionSummary(progress)}</p>
      {progress.running && <>
        <progress aria-label="Recording compaction progress" max={Math.max(1, progress.total)} value={progress.completed} />
        <p className="settings-help">{progress.current && recordingName(progress.current)}{progress.blocksTotal > 0
          ? progress.blocksCompleted === progress.blocksTotal ? ' Â· Verifying recordingâ€¦' : ` Â· Block ${progress.blocksCompleted + 1} of ${progress.blocksTotal}` : ''}</p>
      </>}
      {progress.unchanged.length > 0 && <details><summary>Recordings left unchanged</summary>
        <ul>{progress.unchanged.map((failure, index) => <li key={`${failure.path}:${index}`}>{recordingName(failure.path)}: {failure.reason}</li>)}</ul>
      </details>}
    </div>}
    {error && <div className="settings-group-body"><p className="map-menu-error" role="alert">{error}</p></div>}
  </>;
}
