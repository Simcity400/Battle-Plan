import { useEffect, useState } from 'react';
import { desktopAvailable, errorMessage, mapAction, recordingName } from './api';
import type { DesktopState, ReplayMetadata } from './api';
import { formatDateTime } from '../dashboard/calendar';
import { Icon } from '../components/Icons';
import { SettingsBlock } from './appearance/AppearanceSettings';
import { useMapDiagnostics } from './diagnosticsSetting';

/** The maps' diagnostics switch, recording coverage and the native map's latest report, collected when the page opens. */
export default function MapDiagnostics({ native, metadata }: { native: DesktopState | null; metadata: ReplayMetadata | null }) {
  const [error, setError] = useState<string | null>(null);
  const details = native?.mapDetails;
  const ready = desktopAvailable && !!native?.ready;
  const inspect = () => { setError(null); void mapAction({ kind: 'inspect' }).catch(cause => setError(errorMessage(cause))); };
  useEffect(() => { if (ready) inspect(); }, [ready]);
  const overlay = useMapDiagnostics();
  return <div className="diagnostics-settings">
    <SettingsBlock id="diagnostics-overlay" title="On the map">
      <label className="map-option-row"><span>Show diagnostics on maps <span className="folder-info" tabIndex={0} role="img" aria-label="About map diagnostics" data-tooltip="Technical stats and data limitations over the campaign map and map widgets. Off for normal viewing. F3 toggles them anywhere in the app."><Icon name="info" /></span></span>
        {desktopAvailable ? <input type="checkbox" role="switch" disabled={overlay.enabled === null} aria-busy={overlay.enabled === null} checked={overlay.enabled ?? false} onChange={event => overlay.set(event.currentTarget.checked)} /> : <span className="graphics-unavailable">Desktop only</span>}
      </label>
      {overlay.error && <div className="settings-group-body"><p className="map-menu-error" role="alert">{overlay.error}</p></div>}
    </SettingsBlock>
    <SettingsBlock id="diagnostics-recording" title="Recording">
      {metadata ? <>
        <div className="settings-row"><span className="settings-row-text"><strong data-tooltip={metadata.path}>{recordingName(metadata.path)}</strong>
          <small>Recording format {metadata.format} · {metadata.branches.length} {metadata.branches.length === 1 ? 'branch' : 'branches'}</small></span></div>
        {(metadata.warnings.length > 0 || metadata.branches.some(branch => branch.gaps == null)) && <div className="settings-group-body diagnostics-list">
          {metadata.warnings.map((warning, index) => <p key={index}>{warning}</p>)}
          {metadata.branches.some(branch => branch.gaps == null) && <p>Gap metadata is unavailable from this bridge version. It needs a rebuild after benchmarking.</p>}
        </div>}
        {metadata.branches.map(branch => <details key={branch.id} className="settings-row diagnostics-branch"><summary><span className="settings-row-text"><strong>Branch {branch.id}</strong>
          <small>{formatDateTime(branch.firstHour)} · session {branch.sessionId} · {branch.sampleBoundary}</small></span>
          <span className={`pill${(branch.gaps ?? []).length ? ' warning' : ''}`}>{branch.gaps == null ? 'Gaps unavailable' : `${branch.gaps.length} ${branch.gaps.length === 1 ? 'gap' : 'gaps'}`}</span></summary>
          {(branch.gaps ?? []).length === 0 ? <p>No reported capture gaps{branch.gaps == null ? ' available' : ''}.</p>
            : <ul>{branch.gaps.map((gap, index) => <li key={index}>{formatDateTime(gap.firstHour)} – {formatDateTime(gap.lastHour)}: {gap.reason}</li>)}</ul>}
        </details>)}
      </> : <div className="settings-row"><span className="settings-row-text"><strong>No recording selected</strong><small>Load a campaign to see its coverage and reported gaps.</small></span></div>}
    </SettingsBlock>
    <SettingsBlock id="diagnostics-map" title="Native map" actions={ready && <button type="button" className="button small quiet" onClick={inspect}><Icon name="refresh" />Update report</button>}>
      <div className="settings-row"><span className="settings-row-text"><strong>{details?.path ? recordingName(details.path) : 'No map loaded'}</strong>
        <small>{!desktopAvailable ? 'The native map runs in the desktop app.' : details?.hour != null ? formatDateTime(details.hour) : 'The most recent native map report, collected on demand.'}</small></span></div>
      {error && <div className="settings-group-body"><p className="map-menu-error" role="alert">{error}</p></div>}
      {details && details.messages.length > 0 && <div className="settings-group-body diagnostics-list"><ul>{details.messages.map((message, index) => <li key={index}>{message}</li>)}</ul></div>}
    </SettingsBlock>
  </div>;
}
