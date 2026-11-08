// Battle Plan's Updates settings: the version with its update action, and the channel both apps
// follow, as T3 Code's version and update track rows.
import { useState } from 'react';
import { Icon } from '../components/Icons';
import { Select } from '../components/Select';
import type { UpdateChannel } from './api';
import * as api from './api';
import { CHANNEL_LABELS, channelDescription, confirmInstall, updateAction, updateButtonLabel, updateDescription, updateDisabled } from './logic';
import { UpdateNotes, useUpdateActions } from './UpdateControl';
import { useUpdateState } from './useUpdateState';

export default function UpdateSettings() {
  const { state, error } = useUpdateState();
  const actions = useUpdateActions();
  // The version the restart question is about: it no longer shows once that download is gone.
  const [confirming, setConfirming] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  if (!api.updatesAvailable) return <p className="settings-row-text"><small>Updates are part of the desktop app.</small></p>;
  const action = updateAction(state);
  const version = state?.downloadedVersion ?? null;
  const asking = confirming !== null && confirming === version;
  const press = () => {
    if (action === 'check') void actions.check();
    if (action === 'download') void actions.download();
    if (action === 'install') setConfirming(version);
  };
  const choose = (channel: string) => {
    if (!state || channel === state.channel || (channel !== 'stable' && channel !== 'nightly')) return;
    setSaving(true);
    void actions.run('Could not change the update channel', () => api.setUpdateChannel(channel as UpdateChannel)).finally(() => setSaving(false));
  };
  return <>
    <div className="settings-row" id="updates-version"><span className="settings-row-text">
      <strong>Battle Plan {state?.currentVersion ?? ''}</strong>
      <small>{error ?? updateDescription(state)}</small></span>
      <button type="button" className={`button small${action === 'install' ? ' primary' : ''}`} disabled={updateDisabled(state) || actions.pending || asking}
        onClick={press}><Icon name={action === 'download' ? 'download' : 'refresh-cw'} />{updateButtonLabel(state)}</button>
    </div>
    {asking && state && version && <div className="settings-row update-confirm" role="alertdialog" aria-labelledby="update-confirm-title">
      <span className="settings-row-text"><strong id="update-confirm-title">Restart to update to {version}?</strong>
        <small>{state.installBlocked ?? confirmInstall(version)}</small><UpdateNotes state={state} /></span>
      <span className="update-confirm-actions">
        <button type="button" className="button small quiet" onClick={() => void actions.openRelease(version)}>Release notes</button>
        <button type="button" className="button small quiet" autoFocus onClick={() => setConfirming(null)}>Cancel</button>
        <button type="button" className="button small primary" disabled={state.installBlocked != null || actions.pending}
          onClick={() => { setConfirming(null); void actions.install(); }}>Restart to update</button>
      </span>
    </div>}
    <div className="settings-row" id="updates-channel"><span className="settings-row-text">
      <strong>Update channel</strong><small>{channelDescription(state)}</small></span>
      <Select label="Update channel" value={state?.channel ?? ''} disabled={!state?.enabled || saving || actions.pending || state.status === 'downloading'}
        options={state ? (['stable', 'nightly'] as const).map(value => ({ value, label: CHANNEL_LABELS[value] })) : [{ value: '', label: 'Loading…' }]}
        onChange={choose} />
    </div>
    {actions.failure && <p className="map-menu-error" role="alert">{actions.failure.title}: {actions.failure.detail}
      <button type="button" className="button quiet" onClick={actions.dismiss}>Dismiss</button></p>}
  </>;
}
