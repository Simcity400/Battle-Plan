import { useEffect, useState } from 'react';
import { Select } from '../components/Select';
import type { DashboardClient, RecorderSettings } from '../dashboard/client';
import { AUTO_COMPRESS_NOTE, COMPRESSION_NOTE } from '../recorder/compression';
import type { Compression } from '../recorder/compression';
import { errorMessage } from './errors';

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
    <div className="settings-row"><span className="settings-row-text"><strong>Compression</strong>
      <small>{COMPRESSION_NOTE} The recorder compresses outside the game, never in it. Changes apply to the next block.</small></span>
      <Select label="Recording compression" value={value ?? ''} disabled={loading || saving || value === null || !canCompress}
        options={[...(value ? [] : [{ value: '', label: loading ? 'Loading...' : 'Unavailable' }]), { value: 'low', label: 'Low' }, { value: 'high', label: 'High' }]} onChange={value => onCompression(value === 'high' ? 'high' : 'low')} />
    </div>
    {settings?.liveCompression === 'low' && <label className="settings-row"><span className="settings-row-text"><strong>Auto Compress</strong>
      <small>{AUTO_COMPRESS_NOTE}</small></span>
      <input type="checkbox" role="switch" aria-label="Auto Compress" checked={settings.autoCompress} disabled={saving || !canAutoCompress}
        onChange={event => onAutoCompress(event.currentTarget.checked)} /></label>}
  </>;
}

export default function RecordingCompressionSetting({ client }: { client: DashboardClient }) {
  const [settings, setSettings] = useState<RecorderSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [retry, setRetry] = useState(0);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    if (!client.recorderSettings) { setLoading(false); return; }
    setLoading(true); setError(null);
    void client.recorderSettings().then(settings => { if (alive) setSettings(settings); })
      .catch(cause => { if (alive) setError(errorMessage(cause)); }).finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [client, retry]);
  const save = async (change: () => Promise<RecorderSettings>) => {
    setSaving(true); setError(null);
    try { setSettings(await change()); }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setSaving(false); }
  };
  return <>
    <CompressionChoice settings={settings} loading={loading} saving={saving} canCompress={!!client.setRecordingCompression} canAutoCompress={!!client.setAutoCompress}
      onCompression={compression => { const set = client.setRecordingCompression?.bind(client); if (set) void save(() => set(compression)); }}
      onAutoCompress={enabled => { const set = client.setAutoCompress?.bind(client); if (set) void save(() => set(enabled)); }} />
    {error && <p className="map-menu-error" role="alert">{error}{settings === null && <button type="button" className="button quiet" onClick={() => setRetry(count => count + 1)}>Retry</button>}</p>}
  </>;
}
