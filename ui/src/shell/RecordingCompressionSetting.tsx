import { useEffect, useState } from 'react';
import { Segmented } from '../components/Segmented';
import type { DashboardClient, RecorderSettings } from '../dashboard/client';
import { AUTO_COMPRESS_NOTE, COMPRESSION_NOTE } from '../recorder/compression';
import type { Compression } from '../recorder/compression';
import { errorMessage } from './errors';
import { Icon } from '../components/Icons';
import { FailureLine } from '../components/FailureLine';

export const COMPRESSION_DETAIL = 'Compression runs outside the game and applies to what is recorded next.';
export const AUTO_COMPRESS_HIGH = 'Only with Low compression. High recordings are already compressed as they’re made.';

export interface CompressionChoiceProps {
  settings: Pick<RecorderSettings, 'liveCompression' | 'autoCompress'> | null;
  loading: boolean;
  saving: boolean;
  canCompress: boolean;
  canAutoCompress: boolean;
  onCompression(compression: Compression): void;
  onAutoCompress(enabled: boolean): void;
}

export function CompressionChoice({ settings, loading, saving, canCompress, canAutoCompress, onCompression, onAutoCompress }: CompressionChoiceProps) {
  const value = settings?.liveCompression ?? null;
  return <>
    <div className="settings-row" id="recording-compression"><span className="settings-row-text"><strong className="folder-field-title">Recording compression
      <span className="folder-info" role="img" tabIndex={0} aria-label="About recording compression" data-tooltip={COMPRESSION_DETAIL}><Icon name="info" /></span></strong>
      <small>{COMPRESSION_NOTE}</small></span>
      {value ? <Segmented<Compression> label="Recording compression" value={value} disabled={saving || !canCompress} onChange={next => { if (next !== value) onCompression(next); }}
        options={[{ value: 'low', label: 'Low' }, { value: 'high', label: 'High' }]} />
        : <span className="settings-value" role="status">{loading ? 'Loading…' : canCompress ? 'Unavailable' : 'Desktop only'}</span>}
    </div>
    {settings?.liveCompression === 'low' ? <label className="settings-row" id="recording-auto-compress"><span className="settings-row-text"><strong>Auto compress</strong>
      <small>{AUTO_COMPRESS_NOTE}</small></span>
      <input type="checkbox" role="switch" aria-label="Auto compress" checked={settings.autoCompress} disabled={saving || !canAutoCompress}
        onChange={event => onAutoCompress(event.currentTarget.checked)} /></label>
      : settings && <div className="settings-row" id="recording-auto-compress"><span className="settings-row-text"><strong>Auto compress</strong>
        <small>{AUTO_COMPRESS_HIGH}</small></span><span className="settings-value">Low only</span></div>}
  </>;
}

export default function RecordingCompressionSetting({ client }: { client: DashboardClient }) {
  const [settings, setSettings] = useState<RecorderSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [retry, setRetry] = useState(0);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<{ text: string; detail: string; retry: () => void } | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    if (!client.recorderSettings) { setLoading(false); return; }
    setLoading(true); setError(null);
    void client.recorderSettings().then(settings => { if (alive) setSettings(settings); })
      .catch(cause => { if (alive) setError({ text: 'Couldn’t read the recorder settings.', detail: errorMessage(cause), retry: () => setRetry(count => count + 1) }); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [client, retry]);
  const save = async (change: () => Promise<RecorderSettings>) => {
    setSaving(true); setError(null); setWarning(null);
    try { const saved = await change(); setSettings(saved); setWarning(saved.warning ?? null); }
    catch (cause) { setError({ text: 'Couldn’t save that change.', detail: errorMessage(cause), retry: () => void save(change) }); }
    finally { setSaving(false); }
  };
  return <>
    <CompressionChoice settings={settings} loading={loading} saving={saving} canCompress={!!client.setRecordingCompression} canAutoCompress={!!client.setAutoCompress}
      onCompression={compression => { const set = client.setRecordingCompression?.bind(client); if (set) void save(() => set(compression)); }}
      onAutoCompress={enabled => { const set = client.setAutoCompress?.bind(client); if (set) void save(() => set(enabled)); }} />
    {error && <FailureLine className="settings-failure" message={error.text} detail={error.detail} onRetry={error.retry} retryDisabled={saving || loading} />}
    {!error && warning && <div className="notice warning" role="status" data-detail={warning}><Icon name="alert" /><span>Saved, but Windows didn’t confirm the change reached the disk.</span></div>}
  </>;
}
