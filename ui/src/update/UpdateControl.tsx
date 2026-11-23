import { createContext, useContext, useEffect, useRef, useState } from 'react';
import { FailureLine } from '../components/FailureLine';
import { Icon } from '../components/Icons';
import { Popover, PopoverBody, usePopover } from '../components/Popover';
import { errorMessage } from '../shell/errors';
import * as api from './api';
import type { UpdateState } from './api';
import { actionFailure } from '../dashboard/queryError';
import { blockedReason, confirmInstall, releaseHeading, releaseView, restartLabel, restartQuestion, updateAction, updateDisabled, updateFailure, updateNews,
  updateTooltip } from './logic';
import './update.css';

const RING_RADIUS = 14;
const RING = 2 * Math.PI * RING_RADIUS;

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
  const failed = updateFailure(state) != null;
  const offered = !failed && (action === 'install' || action === 'download');
  return <span className={`update-icon${state?.status === 'checking' ? ' is-checking' : ''}`}>
    <Icon name="download" />{offered && <span className="update-badge update-dot" />}{failed && <span className="update-badge is-alert" />}</span>;
}

export interface ActionFailure { text: string; detail?: string; retry(): void }

export type RestartGuard = (restart: () => Promise<boolean>, opener: HTMLElement | null) => void;

export const RestartGuardContext = createContext<RestartGuard | null>(null);

export function useGuardedRestart(): RestartGuard {
  const guard = useContext(RestartGuardContext);
  return (restart, opener) => { if (guard) guard(restart, opener); else void restart(); };
}

const shownByState = (state: UpdateState) => updateFailure(state) != null;

export function useUpdateActions() {
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<ActionFailure | null>(null);
  const guard = useGuardedRestart();
  const run = async (title: string, work: () => Promise<unknown>, tracked = false, again?: () => void): Promise<boolean> => {
    setPending(true);
    setFailure(null);
    try { await work(); return true; }
    catch (cause) {
      const shown = tracked && await api.updateState().then(shownByState, () => false);
      if (!shown) setFailure({ ...actionFailure(title, errorMessage(cause)), retry: again ?? (() => void run(title, work, tracked)) });
      return false;
    }
    finally { setPending(false); }
  };
  const install = (opener: HTMLElement | null = null): void =>
    guard(() => run('Couldn’t install the update.', api.installUpdate, true, () => install(opener)), opener);
  return {
    pending, failure, run, install, dismiss: () => setFailure(null),
    check: () => run('Couldn’t check for updates.', api.checkForUpdate, true),
    download: () => run('Couldn’t download the update.', api.downloadUpdate, true),
    openRelease: (version: string) => run('Couldn’t open the release notes.', () => api.openRelease(version)),
  };
}

const PEEK_OPEN_MS = 200;
const PEEK_CLOSE_MS = 150;

export function UpdateNotes({ state }: { state: UpdateState }) {
  if (!state.releases.length && !state.unlisted) return null;
  const now = Date.now();
  return <div className="update-releases">
    {state.releases.map(notes => {
      const { items, footer } = releaseView(notes);
      return <section key={notes.version} className="update-release">
        <h3 className="update-release-heading">{releaseHeading(notes, now)}</h3>
        {items.length > 0 && <ul className="update-notes">
          {items.map((item, index) => <li key={index}>{item.label && <><strong>{item.label}.</strong>{' '}</>}{item.text}</li>)}
        </ul>}
        {footer && <p className="update-notes-more">{footer}</p>}
      </section>;
    })}
    {state.unlisted && <p className="update-notes-more">{state.releases.length ? 'Earlier releases aren’t listed here.' : 'This update’s changes aren’t listed here.'}</p>}
  </div>;
}

export function UpdateButton({ state }: { state: UpdateState | null }) {
  const actions = useUpdateActions();
  const menu = usePopover('start');
  const [notice, setNotice] = useState<{ kind: 'confirm' | 'downloaded'; version: string } | null>(null);
  const [peeking, setPeeking] = useState(false);
  const peekTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const action = updateAction(state);
  const disabled = updateDisabled(state) || actions.pending;
  const downloaded = state?.downloadedVersion ?? null;
  const open = () => { if (menu.trigger.current) menu.openAt(menu.trigger.current.getBoundingClientRect()); };
  useEffect(() => { if (actions.failure) { setNotice(null); open(); } }, [actions.failure]);
  const stale = notice !== null && notice.version !== downloaded;
  useEffect(() => {
    if (!stale) return;
    setNotice(null);
    if (!actions.failure) menu.close();
  }, [stale, actions.failure, menu]);
  useEffect(() => () => clearTimeout(peekTimer.current), []);
  if (!api.updatesAvailable || !(updateNews(state) || menu.open || actions.pending || actions.failure || notice)) return null;
  const offered = state?.downloadedVersion ?? state?.availableVersion ?? null;
  const peekable = !!state && offered != null && (state.releases.length > 0 || state.unlisted) && !actions.failure && !notice;
  const peek = () => {
    clearTimeout(peekTimer.current);
    if (peekable && !menu.open) peekTimer.current = setTimeout(() => { setPeeking(true); open(); }, PEEK_OPEN_MS);
  };
  const unpeek = () => {
    clearTimeout(peekTimer.current);
    if (peeking && peekable) peekTimer.current = setTimeout(() => menu.popup.current?.hidePopover(), PEEK_CLOSE_MS);
  };
  const click = () => {
    if (disabled) return;
    if (action === 'check') void actions.check();
    const offered = state?.availableVersion ?? null;
    if (action === 'download' && offered) void actions.download().then(done => { if (done) { setNotice({ kind: 'downloaded', version: offered }); open(); } });
    if (action === 'install' && downloaded) { setNotice({ kind: 'confirm', version: downloaded }); open(); }
  };
  const tooltip = updateTooltip(state);
  return <>
    <button ref={menu.trigger} type="button" className={`update-button${action === 'download' || action === 'install' || state?.status === 'downloading' ? ' is-offered' : ''}`}
      aria-label={tooltip} aria-disabled={disabled || undefined} data-tooltip={menu.open || peekable ? undefined : tooltip} onClick={click}
      onPointerEnter={peek} onPointerLeave={unpeek}>
      <UpdateIcon state={state} />
    </button>
    <Popover menu={menu} className="update-popover" aria-label="Update" onPointerEnter={() => clearTimeout(peekTimer.current)} onPointerLeave={unpeek}
      onToggle={event => { if (event.newState !== 'open') { setPeeking(false); setNotice(null); } }}>
      <PopoverBody open={menu.open}>{() => {
        if (actions.failure) {
          const { retry } = actions.failure;
          return <>
            <FailureLine className="update-popover-failure" message={actions.failure.text} detail={actions.failure.detail}
              onRetry={() => { actions.dismiss(); menu.close(); retry(); }} retryDisabled={actions.pending} />
            <div className="update-popover-actions">
              <button type="button" className="button small quiet" data-popover-autofocus onClick={() => { actions.dismiss(); menu.close(); }}>Close</button>
            </div>
          </>;
        }
        if (state && peeking && !notice && offered) return <>
          <strong className="update-popover-title">{tooltip}</strong>
          <UpdateNotes state={state} />
          {state.enabled && <div className="update-popover-actions">
            <button type="button" className="button small quiet update-popover-link" onClick={() => void actions.openRelease(offered)}>Release notes</button>
          </div>}
        </>;
        if (!state || !notice || stale) return null;
        const { version } = notice;
        const blocked = state.installBlocked;
        return <>
          <strong className="update-popover-title">{restartQuestion(version, state.downgrade)}</strong>
          <p className="update-popover-text">{blocked ? blockedReason(state) : confirmInstall(version, state.downgrade)}</p>
          <UpdateNotes state={state} />
          <div className="update-popover-actions">
            {state.enabled && <button type="button" className="button small quiet update-popover-link" onClick={() => void actions.openRelease(version)}>Release notes</button>}
            <span className="update-grow" />
            <button type="button" className="button small quiet" data-popover-autofocus onClick={menu.close}>{notice.kind === 'downloaded' ? 'Later' : 'Cancel'}</button>
            <button type="button" className="button small primary" disabled={blocked != null || actions.pending}
              onClick={() => { menu.close(); actions.install(menu.trigger.current); }}>{restartLabel(state)}</button>
          </div>
        </>;
      }}</PopoverBody>
    </Popover>
  </>;
}
