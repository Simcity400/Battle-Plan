// The update button both apps show, after T3 Code's: a check icon that spins while checking, a
// download icon with a dot when an update waits, a progress ring while downloading, and a restart
// icon with a check once downloaded. Restarting asks first; a failure says why.
import { useEffect, useState } from 'react';
import { Icon } from '../components/Icons';
import { Popover, PopoverBody, usePopover } from '../components/Popover';
import { errorMessage } from '../shell/errors';
import * as api from './api';
import type { UpdateState } from './api';
import { confirmInstall, shownNotes, updateAction, updateDisabled, updateTooltip } from './logic';
import './update.css';

const RING_RADIUS = 14;
const RING = 2 * Math.PI * RING_RADIUS;

/** The button's face for a state. */
export function UpdateIcon({ state }: { state: UpdateState | null }) {
  const action = updateAction(state);
  if (state?.status === 'downloading') {
    const percent = Math.min(100, Math.max(0, state.downloadPercent ?? 0));
    return <span className="update-icon is-downloading">
      <svg className="update-ring" viewBox="0 0 32 32" aria-hidden="true">
        <circle cx="16" cy="16" r={RING_RADIUS} className="update-ring-track" />
        <circle cx="16" cy="16" r={RING_RADIUS} className="update-ring-done" strokeDasharray={RING} strokeDashoffset={RING * (1 - percent / 100)} />
      </svg>
      <Icon name="download" />
    </span>;
  }
  if (action === 'install') return <span className="update-icon"><Icon name="refresh-cw" /><span className="update-badge is-ready"><Icon name="check" /></span></span>;
  if (action === 'download') return <span className="update-icon"><Icon name="download" /><span className="update-badge is-dot" /></span>;
  const failed = state?.status === 'error';
  return <span className={`update-icon${state?.status === 'checking' ? ' is-checking' : ''}`}>
    <Icon name="refresh-cw" />{failed && <span className="update-badge is-alert" />}</span>;
}

/** Runs the button's actions; `failure` holds why the last one failed, until dismissed. */
export function useUpdateActions() {
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<{ title: string; detail: string } | null>(null);
  const run = async (title: string, work: () => Promise<unknown>) => {
    setPending(true);
    setFailure(null);
    try { await work(); return true; }
    catch (cause) { setFailure({ title, detail: errorMessage(cause) }); return false; }
    finally { setPending(false); }
  };
  return {
    pending, failure, run, dismiss: () => setFailure(null),
    check: () => run('Could not check for updates', api.checkForUpdate),
    download: () => run('Could not download the update', api.downloadUpdate),
    // Started, the installer closes both apps; an answer means it did not start.
    install: () => run('Could not install the update', api.installUpdate),
    openRelease: (version: string) => run('Could not open the release notes', () => api.openRelease(version)),
  };
}

/** The offered release's changes, as T3 Code lists them beside its update button. */
export function UpdateNotes({ state }: { state: UpdateState }) {
  const { items, more } = shownNotes(state.notes);
  if (!items.length) return null;
  return <ul className="update-notes">
    {items.map((item, index) => <li key={index}>{item}</li>)}
    {more > 0 && <li className="update-notes-more">And {more} more</li>}
  </ul>;
}

/** Battle Plan's sidebar update button, with its notice beside it. */
export function UpdateButton({ state }: { state: UpdateState | null }) {
  const actions = useUpdateActions();
  const menu = usePopover('start');
  // A notice is about one download: it closes once that download is gone, as after a channel change.
  const [notice, setNotice] = useState<{ kind: 'confirm' | 'downloaded'; version: string } | null>(null);
  const action = updateAction(state);
  const disabled = updateDisabled(state) || actions.pending;
  const downloaded = state?.downloadedVersion ?? null;
  const open = () => { if (menu.trigger.current) menu.openAt(menu.trigger.current.getBoundingClientRect()); };
  useEffect(() => { if (actions.failure) { setNotice(null); open(); } }, [actions.failure]); // eslint-disable-line react-hooks/exhaustive-deps
  const stale = notice !== null && notice.version !== downloaded;
  useEffect(() => {
    if (!stale) return;
    setNotice(null);
    if (!actions.failure) menu.close();
  }, [stale, actions.failure, menu]);
  if (!api.updatesAvailable) return null;
  const click = () => {
    if (disabled) return;
    if (action === 'check') void actions.check();
    // A finished download is announced beside the button, as T3 Code's toast does.
    const offered = state?.availableVersion ?? null;
    if (action === 'download' && offered) void actions.download().then(done => { if (done) { setNotice({ kind: 'downloaded', version: offered }); open(); } });
    if (action === 'install' && downloaded) { setNotice({ kind: 'confirm', version: downloaded }); open(); }
  };
  const tooltip = updateTooltip(state);
  return <>
    <button ref={menu.trigger} type="button" className={`update-button${action === 'download' || action === 'install' || state?.status === 'downloading' ? ' is-offered' : ''}`}
      aria-label={tooltip} aria-disabled={disabled || undefined} data-tooltip={menu.open ? undefined : tooltip} onClick={click}>
      <UpdateIcon state={state} />
    </button>
    <Popover menu={menu} className="update-popover" aria-label="Update">
      <PopoverBody open={menu.open}>{() => {
        if (actions.failure) return <>
          <strong className="update-popover-title">{actions.failure.title}</strong>
          <p className="update-popover-text" role="alert">{actions.failure.detail}</p>
          <div className="update-popover-actions">
            <button type="button" className="button small quiet" onClick={() => { actions.dismiss(); menu.close(); }}>Close</button>
            {action !== 'none' && !updateDisabled(state) && <button type="button" className="button small primary" data-popover-autofocus
              onClick={() => { actions.dismiss(); menu.close(); click(); }}>Try again</button>}
          </div>
        </>;
        if (!state || !notice || stale) return null;
        const { version } = notice;
        const blocked = state.installBlocked;
        return <>
          <strong className="update-popover-title">{notice.kind === 'downloaded' ? `Update ${version} downloaded` : `Restart to update to ${version}?`}</strong>
          <p className="update-popover-text">{blocked ?? (notice.kind === 'downloaded' ? 'Restart from the update button to install it in both apps.' : confirmInstall(version))}</p>
          <UpdateNotes state={state} />
          <div className="update-popover-actions">
            {state.enabled && <button type="button" className="button small quiet update-popover-link" onClick={() => void actions.openRelease(version)}>Release notes</button>}
            <span className="update-grow" />
            <button type="button" className="button small quiet" onClick={menu.close}>{notice.kind === 'downloaded' ? 'Later' : 'Cancel'}</button>
            <button type="button" className="button small primary" data-popover-autofocus disabled={blocked != null || actions.pending}
              onClick={() => { menu.close(); void actions.install(); }}>Restart to update</button>
          </div>
        </>;
      }}</PopoverBody>
    </Popover>
  </>;
}
