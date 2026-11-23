import { useEffect, useRef, useState } from 'react';
import { errorMessage } from './api';
import { actionFailure } from '../dashboard/queryError';
import type { ContentFolders } from './api';
import type { DashboardClient } from '../dashboard/client';
import { Icon } from '../components/Icons';
import { FailureLine } from '../components/FailureLine';
import { settingDraft } from './pendingWrites';

type Kind = 'game' | 'user';
export interface FolderFailure { field: Kind | null; text: string; detail?: string; retry?: () => void }

const namedFolder = (message: string): Kind | null => {
  const game = /\bgame folder\b|hoi4\.exe/i.test(message), user = /\buser folder\b/i.test(message);
  return game === user ? null : game ? 'game' : 'user';
};

const REFUSAL: Record<Kind, string> = { game: 'Choose the folder the game is installed in.', user: 'Choose your Paradox user folder.' };

export function folderFailure(message: string, fallback = 'Couldn’t save these folders.'): FolderFailure {
  const field = namedFolder(message);
  return { ...actionFailure(field ? REFUSAL[field] : fallback, message), field };
}

const retryable = (failure: FolderFailure) => !!failure.detail && !(failure.field && failure.text === REFUSAL[failure.field]);

export function FolderField({ label, help, value, placeholder, disabled, readOnly, error, hint, onRetry, onChange, onCommit, onBrowse }: {
  label: string; help: string; value: string; placeholder: string; disabled: boolean; readOnly?: boolean; error?: FolderFailure | null; hint?: string | null;
  onRetry?: () => void; onChange: (value: string) => void; onCommit: () => void; onBrowse?: () => void;
}) {
  const noteId = `${label.replaceAll(' ', '-').toLowerCase()}-note`;
  const [focused, setFocused] = useState(false);
  const note = error || hint;
  return <div className="folder-setting-row">
    <label className="field"><span className="folder-field-title">{label}<span className="folder-info" role="img" tabIndex={0} aria-label={`About ${label}`} data-tooltip={help}
      onClick={event => event.preventDefault()}><Icon name="info" /></span></span>
      <span className="folder-path">
        <input aria-label={label} value={value} placeholder={placeholder} readOnly={readOnly} disabled={disabled} spellCheck={false}
          className={!focused && value ? 'is-covered' : undefined}
          aria-invalid={error ? true : undefined} aria-describedby={note ? noteId : undefined} data-tooltip-overflow={!focused && value ? value : undefined}
          onChange={event => onChange(event.currentTarget.value)} onFocus={() => setFocused(true)} onBlur={() => { setFocused(false); onCommit(); }}
          onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); onCommit(); } }} />
        {!focused && value && <span className="folder-path-shown" aria-hidden="true"><bdi>{value}</bdi></span>}
      </span></label>
    {onBrowse && <button type="button" className="button folder-browse" disabled={disabled} onClick={onBrowse} aria-label={`Browse for ${label.toLowerCase()}`} data-tooltip="Choose folder"><Icon name="folder" /></button>}
    {error ? <div className="folder-note" id={noteId}><FailureLine message={error.text} detail={error.detail} onRetry={onRetry} retryDisabled={disabled} /></div>
      : hint && <p className="folder-hint folder-note" id={noteId} role="status">{hint}</p>}
  </div>;
}

type Paths = Record<Kind, string>;
const FIELDS: { kind: Kind; label: string; help: string; missing: string }[] = [
  { kind: 'game', label: 'Game installation', help: 'The folder where the game is installed.', missing: 'Still needed: choose the game installation to finish setting up.' },
  { kind: 'user', label: 'Paradox user folder', help: 'The game’s user folder under Documents / Paradox Interactive. Contains settings and mod descriptors.', missing: 'Still needed: choose the Paradox user folder to finish setting up.' },
];
const unsaved = settingDraft<Paths>('folders');

export default function GameFolderSettings({ folders, unread = false, client, onFoldersChange }: { folders: ContentFolders | null; unread?: boolean; client: DashboardClient; onFoldersChange: (folders: ContentFolders) => void }) {
  const saved = (): Paths => ({ game: folders?.game ?? '', user: folders?.user ?? '' });
  const [paths, setShown] = useState<Paths>(() => unsaved.value() ?? saved());
  const [error, setError] = useState<FolderFailure | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ready = !!client.setContentFolders;
  const canBrowse = !!client.chooseContentFolder;
  const setPaths = (next: Paths) => {
    const base = saved();
    unsaved.set(next.game === base.game && next.user === base.user ? null : next);
    setShown(next);
  };
  useEffect(() => { if (folders && !unsaved.value()) setShown({ game: folders.game, user: folders.user }); }, [folders]);
  const applying = useRef(false);
  const apply = async (next: Paths, kind: Kind) => {
    if (applying.current || !client.setContentFolders) return;
    if (!next[kind].trim()) { setPaths({ ...next, [kind]: folders?.[kind] ?? '' }); setError(null); return; }
    const game = next.game.trim(), user = next.user.trim();
    if (folders && game === folders.game && user === folders.user) { setError(null); return; }
    if (!game || !user) { setError(null); return; }
    applying.current = true; setBusy(true); setError(null); setWarning(null);
    try { const applied = await client.setContentFolders(game, user); unsaved.set(null); setWarning(applied.warning ?? null); onFoldersChange(applied); }
    catch (cause) { const failure = folderFailure(errorMessage(cause)); setError({ ...failure, retry: retryable(failure) ? () => void apply(next, kind) : undefined }); }
    finally { applying.current = false; setBusy(false); }
  };
  const browse = async (kind: Kind) => {
    setBusy(true); setError(null);
    let chosen: string | null = null;
    try { chosen = await client.chooseContentFolder!(kind, paths[kind]); }
    catch (cause) { setError({ ...actionFailure('Couldn’t open the folder picker.', errorMessage(cause)), field: kind, retry: () => void browse(kind) }); }
    finally { setBusy(false); }
    if (!chosen) return;
    const next = { ...paths, [kind]: chosen };
    setPaths(next);
    await apply(next, kind);
  };
  const started = FIELDS.some(field => paths[field.kind].trim());
  return <div className="game-folder-form">
    {FIELDS.map(field => <FolderField key={field.kind} label={field.label} help={field.help} value={paths[field.kind]}
      placeholder={folders || unread ? 'Choose folder' : 'Reading configured folders…'} readOnly={!ready} disabled={busy || !ready}
      error={error?.field === field.kind ? error : null} onRetry={error?.field === field.kind ? error.retry : undefined}
      hint={started && !paths[field.kind].trim() && !folders?.[field.kind] ? field.missing : null}
      onChange={value => setPaths({ ...paths, [field.kind]: value })}
      onCommit={() => void apply(paths, field.kind)}
      onBrowse={canBrowse ? () => void browse(field.kind) : undefined} />)}
    {!error && warning && <div className="notice warning" role="status" data-detail={warning}><Icon name="alert" /><span>Folders changed, but Windows didn’t confirm the change reached the disk.</span></div>}
    {error && !error.field && <FailureLine className="folder-form-failure" message={error.text} detail={error.detail} retryDisabled={busy}
      onRetry={error.retry} />}
  </div>;
}
