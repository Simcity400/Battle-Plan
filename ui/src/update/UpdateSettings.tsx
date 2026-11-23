import { useRef, useState } from 'react';
import { FailureLine } from '../components/FailureLine';
import { Icon } from '../components/Icons';
import { Segmented } from '../components/Segmented';
import type { UpdateChannel } from './api';
import * as api from './api';
import { blockedReason, CHANNEL_LABELS, channelDescription, channelLocked, confirmInstall, restartLabel, restartQuestion, UPDATE_READ_FAILED, updateAction, updateButtonLabel,
  updateDescription, updateDisabled, updateFailure } from './logic';
import { UpdateNotes, useUpdateActions } from './UpdateControl';
import { useUpdateState } from './useUpdateState';

export default function UpdateSettings() {
  const { state, error, retry } = useUpdateState();
  const failed = error != null && state == null;
  const actions = useUpdateActions();
  const [confirming, setConfirming] = useState<string | null>(null);
  const opener = useRef<HTMLButtonElement>(null);
  if (!api.updatesAvailable) return <div className="settings-row" id="updates-version"><span className="settings-row-text"><strong>BattlePlan</strong>
    <small>BattlePlan and BattlePlan Recorder update together in the desktop app.</small></span><span className="settings-value">Desktop only</span></div>;
  const action = updateAction(state);
  const version = state?.downloadedVersion ?? null;
  const asking = confirming !== null && confirming === version;
  const press = () => {
    if (action === 'check') void actions.check();
    if (action === 'download') void actions.download();
    if (action === 'install') setConfirming(version);
  };
  const choose = (channel: UpdateChannel) => {
    if (!state || channel === state.channel || channelLocked(state, actions.pending)) return;
    void actions.run('Couldn’t change the update channel.', () => api.setUpdateChannel(channel));
  };
  const cancel = () => { setConfirming(null); requestAnimationFrame(() => (opener.current ?? document.querySelector<HTMLElement>('#updates-version button'))?.focus()); };
  const stateFailure = updateFailure(state);
  const blocking = failed ? { text: UPDATE_READ_FAILED, detail: error, retry } : stateFailure && { ...stateFailure, retry: press };
  const failure = blocking ?? actions.failure;
  return <>
    <div className="settings-row" id="updates-version"><span className="settings-row-text">
      <strong>BattlePlan {state?.currentVersion ?? ''}</strong>
      {failure ? <FailureLine message={failure.text} detail={failure.detail} onRetry={failure.retry} retryDisabled={actions.pending} />
        : <small>{updateDescription(state)}</small>}
      {state && !asking && (state.availableVersion || state.downloadedVersion)
        && <UpdateNotes state={state} onOpenRelease={release => void actions.openRelease(release)} />}</span>
      {!blocking && <button ref={opener} type="button" className={`button small${action === 'install' ? ' primary' : ''}`}
        disabled={updateDisabled(state) || actions.pending || asking} onClick={press}><Icon name="download" />{updateButtonLabel(state)}</button>}
    </div>
    {asking && state && version && <div className="settings-row update-confirm" role="alertdialog" aria-labelledby="update-confirm-title"
      onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); cancel(); } }}>
      <span className="settings-row-text"><strong id="update-confirm-title">{restartQuestion(version, state.downgrade)}</strong>
        <small>{blockedReason(state) ?? confirmInstall(version, state.downgrade)}</small><UpdateNotes state={state} onOpenRelease={release => void actions.openRelease(release)} /></span>
      <span className="update-confirm-actions">
        <button type="button" className="button small quiet" autoFocus onClick={cancel}>Cancel</button>
        <button type="button" className="button small primary" disabled={state.installBlocked != null || actions.pending}
          onClick={() => { setConfirming(null); actions.install(opener.current); }}>{restartLabel(state)}</button>
      </span>
    </div>}
    <div className="settings-row" id="updates-channel"><span className="settings-row-text">
      <strong>Update channel</strong><small>{channelDescription(state)}</small></span>
      {state ? <Segmented<UpdateChannel> label="Update channel" value={state.channel} disabled={channelLocked(state, actions.pending)}
        options={(['stable', 'nightly'] as const).map(value => ({ value, label: CHANNEL_LABELS[value] }))} onChange={choose} />
        : <span className="settings-value" role="status">{failed ? 'Unavailable' : 'Loading…'}</span>}
    </div>
  </>;
}
