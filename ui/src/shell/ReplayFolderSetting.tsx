import { useEffect, useRef, useState } from 'react';
import type { DashboardClient } from '../dashboard/client';
import type { ReplayLibraryState } from '../dashboard/replayLibrary';
import { FolderField } from './GameFolderSettings';
import { actionFailure } from '../dashboard/queryError';
import { errorMessage } from './api';

type Failed = { message: string; fallback: string; retry: () => void };

export default function ReplayFolderSetting({ library, client }: { library: ReplayLibraryState; client: DashboardClient }) {
  const [folder, setFolder] = useState(library.folder ?? '');
  const [error, setError] = useState<Failed | null>(null);
  const [picking, setPicking] = useState(false);
  useEffect(() => setFolder(library.folder ?? ''), [library.folder]);
  const saving = useRef(false);
  const save = async (next: string) => {
    const path = next.trim();
    if (saving.current) return;
    if (!path || path === (library.folder ?? '')) { setError(null); if (!path) setFolder(library.folder ?? ''); return; }
    saving.current = true; setError(null);
    try { const saved = await library.setFolder(path); if (saved) setFolder(saved.folder ?? ''); }
    catch (cause) { setError({ message: errorMessage(cause), fallback: 'Couldn’t use this folder.', retry: () => { setFolder(path); void save(path); } }); }
    finally { saving.current = false; }
  };
  const browse = () => {
    setPicking(true); setError(null);
    void client.chooseReplayFolder!(folder).then(async chosen => { if (chosen) { setFolder(chosen); await save(chosen); } })
      .catch(cause => setError({ message: errorMessage(cause), fallback: 'Couldn’t open the folder picker.', retry: browse })).finally(() => setPicking(false));
  };
  const shown: Failed | null = error ?? (library.error ? { message: library.error, fallback: 'Couldn’t read the recordings folder.', retry: () => void library.refresh() } : null);
  const failure = shown ? actionFailure(shown.fallback, shown.message) : null;
  return <FolderField label="Recordings folder" help="Recordings in this folder and its subfolders appear in your library." value={folder}
    placeholder={client.nativeMaps ? 'Folder containing your recordings' : 'Folder on the computer running BattlePlan'} disabled={library.loading || picking}
    error={failure && { ...failure, field: null }} onRetry={failure?.detail ? shown?.retry : undefined}
    onChange={setFolder} onCommit={() => void save(folder)} onBrowse={client.chooseReplayFolder ? browse : undefined} />;
}
