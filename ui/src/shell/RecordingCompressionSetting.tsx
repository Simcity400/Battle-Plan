import { useEffect, useState } from 'react';
import { Select } from '../components/Select';
import type { DashboardClient } from '../dashboard/client';
import type { Compression } from '../recorder/compression';
import { errorMessage } from './errors';

export default function RecordingCompressionSetting({ client }: { client: DashboardClient }) {
  const [value, setValue] = useState<Compression | null>(null);
  const [loading, setLoading] = useState(true);
  const [retry, setRetry] = useState(0);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    if (!client.recorderSettings) { setLoading(false); return; }
    setLoading(true); setError(null);
    void client.recorderSettings().then(settings => { if (alive) setValue(settings.compression ?? 'low'); })
      .catch(cause => { if (alive) setError(errorMessage(cause)); }).finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [client, retry]);
  const save = async (compression: Compression) => {
    if (!client.setRecordingCompression) return;
    setSaving(true); setError(null);
    try { const settings = await client.setRecordingCompression(compression); setValue(settings.compression ?? 'low'); }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setSaving(false); }
  };
  return <>
    <div className="settings-row"><span className="settings-row-text"><strong>Live compression</strong>
      <small>Low makes slightly larger files. High uses more background CPU and makes the save after Stop slower, especially with few cores. Changes apply to the next block.</small></span>
      <Select label="Recording compression" value={value ?? ''} disabled={loading || saving || value === null || !client.setRecordingCompression}
        options={[...(value ? [] : [{ value: '', label: loading ? 'Loading...' : 'Unavailable' }]), { value: 'low', label: 'Low' }, { value: 'high', label: 'High' }]} onChange={value => { void save(value === 'high' ? 'high' : 'low'); }} />
    </div>
    {error && <p className="map-menu-error" role="alert">{error}{value === null && <button type="button" className="button quiet" onClick={() => setRetry(count => count + 1)}>Retry</button>}</p>}
  </>;
}
