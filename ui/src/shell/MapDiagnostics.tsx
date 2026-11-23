import { useEffect, useState } from 'react';
import { desktopAvailable, errorMessage, mapAction } from './api';
import { recordingDisplayName } from '../dashboard/recordingNames';
import type { DesktopState, ReplayMetadata } from './api';
import type { BranchChoice } from '../dashboard/Dashboard';
import { formatDateTime } from '../dashboard/calendar';
import { Icon } from '../components/Icons';
import { FailureLine } from '../components/FailureLine';
import { SettingsBlock } from './SettingsBlock';
import { useMapDiagnostics } from './diagnosticsSetting';
import { branchCoverage, historyLabels, missingTime } from './diagnosticsData';
import { damagedRecordingText, partialRecordingDetail } from './recordingNotice';

const recordingFolder = (path: string) => path.replace(/[\\/][^\\/]*$/, '');

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
        : <div className="settings-row"><span className="settings-row-text"><strong>No campaign open</strong><small>Open one to see how complete its recording is.</small></span></div>}
    </SettingsBlock>
    <SettingsBlock id="diagnostics-map" title="Map" actions={ready && <button type="button" className="button small quiet" onClick={inspect}><Icon name="refresh" />Check again</button>}>
      <label className="settings-row" id="diagnostics-overlay"><span className="settings-row-text"><strong>Technical overlay</strong>
        <small>Frame rate and map statistics over every map. Ctrl+F3 switches it anywhere.</small></span>
        {!desktopAvailable ? <span className="settings-value">Desktop only</span> : overlay.enabled === null ? <span className="settings-value" role="img" aria-label="Technical overlay: loading">—</span>
          : <input type="checkbox" role="switch" checked={overlay.enabled} onChange={event => overlay.set(event.currentTarget.checked)} />}
      </label>
      {overlay.error ? <div className="settings-group-body"><FailureLine message={overlay.enabled === null ? 'Couldn’t read the technical overlay setting.' : 'Couldn’t switch the technical overlay.'}
        detail={overlay.error} onRetry={overlay.retry} retryDisabled={overlay.pending} /></div>
        : overlay.warning && <div className="settings-group-body"><div className="notice warning" role="status" data-detail={overlay.warning}><Icon name="alert" /><span>Switched, but Windows didn’t confirm the setting was saved.</span></div></div>}
      <div className="settings-row"><span className="settings-row-text"><strong>Problems</strong>
        <small>{!desktopAvailable ? 'Problems the map reports appear here.' : !details?.path ? 'Open the map to check it.'
          : details.messages.length ? `What the map reported${details.hour != null ? ` at ${formatDateTime(details.hour)}` : ''}.` : 'None reported.'}</small></span>
        {!desktopAvailable ? <span className="settings-value">Desktop only</span> : !!details?.messages.length && <span className="pill warning">{details.messages.length}</span>}</div>
      {error && <div className="settings-group-body"><FailureLine message="Couldn’t check the map." detail={error} onRetry={inspect} /></div>}
      {!!details?.messages.length && <div className="settings-group-body diagnostics-list"><ul>{details.messages.map((message, index) => <li key={index}>{message}</li>)}</ul></div>}
    </SettingsBlock>
  </div>;
}

function Recording({ metadata, shown }: { metadata: ReplayMetadata; shown: BranchChoice | null }) {
  const branches = metadata.branches;
  const labels = historyLabels(branches);
  const first = Math.min(...branches.map(branch => branch.firstHour)), last = Math.max(...branches.map(branch => branch.lastHour));
  const gaps = branches.reduce((sum, branch) => sum + branch.gaps.length, 0);
  const losses = branches.flatMap(branch => (branch.eventLosses ?? []).map(loss => ({ branch: branch.id, loss })));
  const status = metadata.damaged ? { tone: 'warning', label: 'Damaged' } : metadata.partial ? { tone: 'warning', label: 'Stopped early' } : gaps ? { tone: 'warning', label: `${gaps} ${gaps === 1 ? 'gap' : 'gaps'}` }
    : losses.length ? { tone: 'warning', label: 'Events incomplete' } : { tone: 'positive', label: 'Complete' };
  return <>
    <div className="settings-row"><span className="settings-row-text"><strong data-tooltip={`${recordingDisplayName(metadata.path)}\n${recordingFolder(metadata.path)}`}>{recordingDisplayName(metadata.path)}</strong>
      <small>{branches.length ? `${formatDateTime(first)} – ${formatDateTime(last)}` : 'No recorded time'}</small></span>
      <span className={`pill ${status.tone}`}>{status.label}</span></div>
    {metadata.damaged && <div className="settings-row"><span className="settings-row-text"><strong>Damaged or modified</strong>
      <small>{[damagedRecordingText(metadata.damaged), ...metadata.damaged.reasons].join(' · ')}</small></span></div>}
    {metadata.partial && <div className="settings-row"><span className="settings-row-text"><strong>Stopped early</strong>
      <small>{`${partialRecordingDetail(metadata.partial).replace(/^p/, 'P')}: ${metadata.partial.reason}`}</small></span></div>}
    {branches.map(branch => {
      const coverage = branchCoverage(branch);
      return <div key={branch.id} className="settings-row diagnostics-coverage">
        <span className="settings-row-text"><strong>{branches.length > 1 ? labels.get(branch.id) : 'Coverage'}</strong>
          <small>{branch.gaps.length ? `${missingTime(coverage.missingHours)} missing in ${branch.gaps.length} ${branch.gaps.length === 1 ? 'gap' : 'gaps'}` : metadata.damaged ? 'No recorded gaps' : 'Recorded throughout'}
            {branches.length > 1 && ` · from ${formatDateTime(branch.firstHour)}`}</small></span>
        <div className="coverage-bar" role="img" aria-label={branch.gaps.length ? `${branch.gaps.length} gaps` : 'No gaps'}>
          {coverage.gaps.map((gap, gapIndex) => <span key={gapIndex} className="coverage-gap" style={{ left: `${gap.left}%`, width: `${gap.width}%` }}
            data-tooltip={`${formatDateTime(gap.firstHour)} – ${formatDateTime(gap.lastHour)}\n${gap.reason}`} />)}
        </div>
        {branches.length > 1 && shown && (shown.id === branch.id ? <span className="pill accent coverage-action">Showing</span>
          : <button type="button" className="button small quiet coverage-action" aria-label={`Show ${labels.get(branch.id)}`} onClick={() => shown.choose(branch.id)}>Show</button>)}
      </div>;
    })}
    {losses.length > 0 && <details className="settings-details">
      <summary><span className="settings-row-text"><strong>Event losses</strong>
        <small>{losses.length} {losses.length === 1 ? 'interval' : 'intervals'} between two reads whose recorded events are incomplete; a lost frame is listed as a gap.</small></span>
        <Icon name="chevron-down" className="settings-details-chevron" /></summary>
      <div className="diagnostics-list"><ul>{losses.map(({ branch, loss }, index) => <li key={index}>
        {branches.length > 1 && `${labels.get(branch)}: `}{formatDateTime(loss.afterHour)} – {formatDateTime(loss.hour)}: {loss.reason}</li>)}</ul></div>
    </details>}
    {metadata.buildWarnings.length > 0 && <details className="settings-details">
      <summary><span className="settings-row-text"><strong>Game version</strong>
        <small>The game version is new to this release of BattlePlan, or differs from the one recorded, so some details may be missing. {metadata.buildWarnings.length} technical {metadata.buildWarnings.length === 1 ? 'note' : 'notes'}.</small></span>
        <Icon name="chevron-down" className="settings-details-chevron" /></summary>
      <div className="diagnostics-list"><ul>{metadata.buildWarnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></div>
    </details>}
    {metadata.warnings.length > 0 && <details className="settings-details">
      <summary><span className="settings-row-text"><strong>File notes</strong>
        <small>{metadata.warnings.length} {metadata.warnings.length === 1 ? 'thing' : 'things'} this file couldn’t provide, such as details only newer versions record.</small></span>
        <Icon name="chevron-down" className="settings-details-chevron" /></summary>
      <div className="diagnostics-list"><ul>{metadata.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></div>
    </details>}
  </>;
}
