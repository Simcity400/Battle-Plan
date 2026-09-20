import { useEffect, useState } from 'react';
import { chooseContentFolder, desktopAvailable, errorMessage, mapAction } from './api';
import type { ContentFolders, DesktopState } from './api';
import type { DashboardClient } from './dashboard/client';
import { Icon } from './Icons';

export default function GameFolderSettings({ folders, native, client, onFoldersChange }: { folders: ContentFolders | null; native: DesktopState | null; client: DashboardClient; onFoldersChange: (folders: ContentFolders) => void }) {
  const [game, setGame] = useState('');
  const [user, setUser] = useState('');
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [submitted, setSubmitted] = useState<{ game: string; user: string } | null>(null);
  const ready = desktopAvailable ? !!native?.ready : !!client.setContentFolders;
  const canBrowse = desktopAvailable || !!client.chooseContentFolder;
  useEffect(() => { if (folders && !dirty) { setGame(folders.game); setUser(folders.user); } }, [folders, dirty]);
  useEffect(() => {
    if (submitted && folders?.contentReady && folders.game === submitted.game && folders.user === submitted.user) {
      setDirty(false); setSubmitted(null);
    }
  }, [folders, submitted]);
  const browse = async (kind: 'game' | 'user', initial: string, set: (path: string) => void) => {
    setBusy(true); setError(null);
    try { const path = desktopAvailable ? await chooseContentFolder(kind, initial) : await client.chooseContentFolder!(kind, initial); if (path) { set(path); setDirty(true); } }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setBusy(false); }
  };
  return <form className="game-folder-form" onSubmit={async event => {
    event.preventDefault(); setBusy(true); setError(null);
    try {
      if (desktopAvailable) {
        await mapAction({ kind: 'folders', game: game.trim(), user: user.trim() });
        setSubmitted({ game: game.trim(), user: user.trim() });
      } else if (client.setContentFolders) {
        const next = await client.setContentFolders(game.trim(), user.trim());
        setDirty(false); onFoldersChange(next);
      }
    }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setBusy(false); }
  }}>
    {[
      { kind: 'game' as const, label: 'the game installation', help: 'The the game folder containing hoi4.exe.', value: game, set: setGame },
      { kind: 'user' as const, label: 'Paradox user folder', help: 'Usually Documents / Paradox Interactive / Hearts of Iron IV. Contains settings and mod descriptors.', value: user, set: setUser },
    ].map(field => <div className="folder-setting-row" key={field.label}>
      <label className="field"><span className="folder-field-title">{field.label}<span className="folder-info" role="img" tabIndex={0} aria-label={`About ${field.label}`} data-tooltip={field.help}><Icon name="info" /></span></span><input aria-label={field.label} value={field.value} placeholder={folders ? 'Choose folder' : 'Reading configured folders…'} readOnly={!ready}
        disabled={busy || (desktopAvailable && !ready)} onChange={event => { field.set(event.currentTarget.value); setDirty(true); }} data-tooltip={field.value || undefined} />
      </label>
      {canBrowse && <button type="button" className="button folder-browse" disabled={!ready || busy} onClick={() => void browse(field.kind, field.value, field.set)} aria-label={`Browse ${field.label}`} data-tooltip="Choose folder"><Icon name="folder" /></button>}
    </div>)}
    {dirty && <button type="submit" className="button folder-apply" disabled={!ready || busy || !game.trim() || !user.trim()}>Apply game folders</button>}
    {(error || native?.mapDetails?.error) && <p className="map-menu-error" role="alert">{error || native?.mapDetails?.error}</p>}
  </form>;
}
