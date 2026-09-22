import { useState } from 'react';
import { desktopAvailable, errorMessage, mapAction, recordingName } from './api';
import type { DesktopState, ReplayMetadata } from './api';
import { formatDateTime } from '../dashboard/calendar';
import { Icon } from '../components/Icons';

export default function MapDiagnostics({ native, metadata }: { native: DesktopState | null; metadata: ReplayMetadata | null }) {
  const [error, setError] = useState<string | null>(null);
  const details = native?.mapDetails;
  const ready = desktopAvailable && !!native?.ready;
  const inspect = () => { setError(null); void mapAction({ kind: 'inspect' }).catch(cause => setError(errorMessage(cause))); };
  return <section className="settings-section">
    <details className="map-diagnostics" onToggle={event => { if (event.currentTarget.open && ready) inspect(); }}><summary>Diagnostics</summary><div>
      <p className="settings-help">Recording coverage and the most recent native map report. Collected on demand.</p>
      {metadata ? <>
        <strong data-tooltip={metadata.path}>{recordingName(metadata.path)}</strong>
        <p>Recording format {metadata.format} · {metadata.branches.length} {metadata.branches.length === 1 ? 'branch' : 'branches'}</p>
        {metadata.warnings.map((warning, index) => <p key={index}>{warning}</p>)}
        {metadata.branches.some(branch => branch.gaps == null) && <p>Gap metadata is unavailable from this bridge version. It needs a rebuild after benchmarking.</p>}
        {metadata.branches.map(branch => <details key={branch.id}><summary>Branch {branch.id} · {formatDateTime(branch.firstHour)}</summary>
          <p>Session {branch.sessionId} · {branch.sampleBoundary}</p>
          {(branch.gaps ?? []).length === 0 ? <p>No reported capture gaps{branch.gaps == null ? ' available' : ''}.</p> : <ul>{branch.gaps.map((gap, index) => <li key={index}>{formatDateTime(gap.firstHour)} – {formatDateTime(gap.lastHour)}: {gap.reason}</li>)}</ul>}
        </details>)}
      </> : <p>No recording selected.</p>}
      {ready && <button type="button" className="button" onClick={inspect}><Icon name="refresh" />Update map report</button>}
      {error && <p className="map-menu-error" role="alert">{error}</p>}
      {details && <><p>Native map: {details.path ? recordingName(details.path) : 'No map loaded'}{details.hour != null ? ` · ${formatDateTime(details.hour)}` : ''}</p>
        <ul>{details.messages.map((message, index) => <li key={index}>{message}</li>)}</ul></>}
    </div></details>
  </section>;
}
