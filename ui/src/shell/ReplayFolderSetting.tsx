import { useEffect, useState } from 'react';
import type { DashboardClient } from '../dashboard/client';
import type { ReplayLibraryState } from '../dashboard/replayLibrary';
import { Icon } from '../components/Icons';

export default function ReplayFolderSetting({ library, client, compact = false }: { compact?: boolean; library: ReplayLibraryState; client: DashboardClient }) {
  const [folder, setFolder] = useState(library.folder ?? '');
  const [error, setError] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);
  useEffect(() => setFolder(library.folder ?? ''), [library.folder]);
  const save = async (folder: string) => { setError(null); try { const saved = await library.setFolder(folder.trim()); if (saved) setFolder(saved.folder ?? ''); } catch (error) { setError(String(error)); } };
  return <form className={`replay-folder-form${compact ? ' compact-folder-form' : ''}`} onSubmit={event => { event.preventDefault(); void save(folder); }}>
    <label className="field"><span className="folder-field-title">Replay folder<span className="folder-info" role="img" tabIndex={0} aria-label="About Replay folder" data-tooltip="Replays in this folder and its subfolders appear in your library."><Icon name="info" /></span></span><input aria-label="Replay folder" value={folder} disabled={library.loading || picking} placeholder="Folder containing your replays" onChange={event => setFolder(event.currentTarget.value)} data-tooltip={folder || undefined} /></label>
    <div className="button-row">
      {client.chooseReplayFolder && <button type="button" className="button" aria-label="Browse replay folder" data-tooltip="Choose folder" disabled={library.loading || picking} onClick={() => {
        setPicking(true); setError(null);
        void client.chooseReplayFolder!(folder).then(async chosen => { if (chosen) { setFolder(chosen); await save(chosen); } }).catch(error => setError(String(error))).finally(() => setPicking(false));
      }}>{compact ? <Icon name="folder" /> : 'Browse…'}</button>}
      {(!compact || folder.trim() !== (library.folder ?? '')) && <button type="submit" className="button" disabled={library.loading || picking || !folder.trim() || folder.trim() === library.folder}>Use folder</button>}
    </div>
    {!client.nativeMaps && !compact && <p className="settings-help">Folder on the computer running Battle Plan.</p>}
    {(error || library.error) && <p className="dash-inline-warning" role="alert">{error || library.error}</p>}
  </form>;
}
