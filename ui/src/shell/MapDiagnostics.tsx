import { useEffect, useState } from 'react';
import { desktopAvailable, errorMessage, mapAction } from './api';
import { recordingDisplayName, recordingNameTooltip, useRecordingNames } from '../dashboard/recordingNames';
import type { DesktopState, ReplayMetadata } from './api';
import type { BranchChoice } from '../dashboard/Dashboard';
import { formatDateTime } from '../dashboard/calendar';
import { Icon } from '../components/Icons';
import { SettingsBlock } from './appearance/AppearanceSettings';
import { useMapDiagnostics } from './diagnosticsSetting';
import { branchCoverage, branchLabels, missingTime } from './diagnosticsData';

/** How complete the open recording is, and the map's troubleshooting tools: its technical
 * overlay and the problems it last reported. */
export default function MapDiagnostics({ native, metadata, branch }: { native: DesktopState | null; metadata: ReplayMetadata | null; branch?: BranchChoice | null }) {
  const [error, setError] = useState<string | null>(null);
  const details = native?.mapDetails;
  const ready = desktopAvailable && !!native?.ready;
  const inspect = () => { setError(null); void mapAction({ kind: 'inspect' }).catch(cause => setError(errorMessage(cause))); };
  useEffect(() => { if (ready) inspect(); }, [ready]);
  const overlay = useMapDiagnostics();
  return <div className="diagnostics-settings">
    <SettingsBlock id="diagnostics-recording" title="Recording">
      {metadata ? <Recording metadata={metadata} shown={branch ?? null} />
        : <div className="settings-row"><span className="settings-row-text"><strong>No recording open</strong><small>Open one to see how complete it is.</small></span></div>}
    </SettingsBlock>
    <SettingsBlock id="diagnostics-map" title="Map" actions={ready && <button type="button" className="button small quiet" onClick={inspect}><Icon name="refresh" />Check again</button>}>
      <label className="settings-row" id="diagnostics-overlay"><span className="settings-row-text"><strong>Technical overlay</strong>
        <small>Frame rate and map statistics over every map. F3 switches it anywhere.</small></span>
        {desktopAvailable ? <input type="checkbox" role="switch" disabled={overlay.enabled === null} aria-busy={overlay.enabled === null} checked={overlay.enabled ?? false}
          onChange={event => overlay.set(event.currentTarget.checked)} /> : <span className="graphics-unavailable">Desktop only</span>}
      </label>
      {overlay.error && <div className="settings-group-body"><p className="map-menu-error" role="alert">{overlay.error}</p></div>}
      <div className="settings-row"><span className="settings-row-text"><strong>Problems</strong>
        <small>{!desktopAvailable ? 'The map runs in the desktop app.' : !details?.path ? 'Open the map to check it.'
          : details.messages.length ? `What the map reported${details.hour != null ? ` at ${formatDateTime(details.hour)}` : ''}.` : 'None reported.'}</small></span>
        {!!details?.messages.length && <span className="pill warning">{details.messages.length}</span>}</div>
      {error && <div className="settings-group-body"><p className="map-menu-error" role="alert">{error}</p></div>}
      {!!details?.messages.length && <div className="settings-group-body diagnostics-list"><ul>{details.messages.map((message, index) => <li key={index}>{message}</li>)}</ul></div>}
    </SettingsBlock>
  </div>;
}

/** The recording's span and status, one coverage row per branch (several can be played
 * from here), and its notes folded away. */
function Recording({ metadata, shown }: { metadata: ReplayMetadata; shown: BranchChoice | null }) {
  const names = useRecordingNames();
  const branches = metadata.branches;
  const labels = branchLabels(branches);
  const first = Math.min(...branches.map(branch => branch.firstHour)), last = Math.max(...branches.map(branch => branch.lastHour));
  const gaps = branches.reduce((sum, branch) => sum + branch.gaps.length, 0);
  const status = metadata.partial ? { tone: 'warning', label: 'Stopped early' } : gaps ? { tone: 'warning', label: `${gaps} ${gaps === 1 ? 'gap' : 'gaps'}` }
    : { tone: 'positive', label: 'Complete' };
  return <>
    <div className="settings-row"><span className="settings-row-text"><strong data-tooltip={`${recordingNameTooltip(metadata.path, names)}\n${metadata.path}`}>{recordingDisplayName(metadata.path, names)}</strong>
      <small>{branches.length ? `${formatDateTime(first)} – ${formatDateTime(last)}` : 'No recorded time'}</small></span>
      <span className={`pill ${status.tone}`}>{status.label}</span></div>
    {metadata.partial && <div className="settings-row"><span className="settings-row-text"><strong>Stopped early</strong>
      <small>{metadata.partial.failedHour != null ? `Nothing after ${formatDateTime(metadata.partial.failedHour)} was saved: ` : 'The end of the recording was not saved: '}{metadata.partial.reason}</small></span></div>}
    {branches.map(branch => {
      const coverage = branchCoverage(branch);
      return <div key={branch.id} className="settings-row diagnostics-coverage">
        <span className="settings-row-text"><strong>{branches.length > 1 ? labels.get(branch.id) : 'Coverage'}</strong>
          <small>{branch.gaps.length ? `${missingTime(coverage.missingHours)} missing in ${branch.gaps.length} ${branch.gaps.length === 1 ? 'gap' : 'gaps'}` : 'Recorded throughout'}
            {branches.length > 1 && ` · from ${formatDateTime(branch.firstHour)}`}</small></span>
        <div className="coverage-bar" role="img" aria-label={branch.gaps.length ? `${branch.gaps.length} gaps` : 'No gaps'}>
          {coverage.gaps.map((gap, gapIndex) => <span key={gapIndex} className="coverage-gap" style={{ left: `${gap.left}%`, width: `${gap.width}%` }}
            data-tooltip={`${formatDateTime(gap.firstHour)} – ${formatDateTime(gap.lastHour)}\n${gap.reason}`} />)}
        </div>
        {branches.length > 1 && shown && (shown.id === branch.id ? <span className="pill accent coverage-action">Showing</span>
          : <button type="button" className="button small quiet coverage-action" aria-label={`Show ${labels.get(branch.id)}`} onClick={() => shown.choose(branch.id)}>Show</button>)}
      </div>;
    })}
    {metadata.warnings.length > 0 && <details className="diagnostics-details">
      <summary><span className="settings-row-text"><strong>File notes</strong>
        <small>{metadata.warnings.length} {metadata.warnings.length === 1 ? 'thing' : 'things'} this file couldn’t provide, such as details only newer versions record.</small></span>
        <Icon name="chevron-down" className="diagnostics-chevron" /></summary>
      <div className="diagnostics-list"><ul>{metadata.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></div>
    </details>}
  </>;
}
