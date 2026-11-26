import { Popover, PopoverHeading, usePopover } from './Popover';
import { Select } from './Select';
import MapAppearance from './MapAppearance';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import type { ReactNode } from 'react';
import { useId, useState } from 'react';
import { desktopAvailable, errorMessage, exportMap } from '../shell/api';
import { closePopout, nextPopoutMonitor, openPopout, restartPopout, setPopoutCameraMode } from '../shell/popout';
import CameraModeControl from './CameraModeControl';
import AircraftPicker from './AircraftPicker';
import type { AircraftChoice } from '../dashboard/aircraftChoice';
import type { CameraMode, FlyAircraftStatus } from '../generated/dashboard';
import type { MapOptions, ReplayMetadata, SavedExport } from '../shell/api';
import { formatDateTime } from '../dashboard/calendar';
import { Icon } from './Icons';
import { FailureLine } from './FailureLine';
import { actionFailure, type PlainFailure } from '../dashboard/queryError.ts';
import type { BranchJoin } from '../generated/dashboard';
import { historyLabels } from '../shell/diagnosticsData';
import { useExportVideo } from '../export/ExportVideoContext';
import './mapControls.css';

export default function MapControls({ source, client, metadata, branchId, onBranch, onChooseHistory, options, onOptions, compact = false, viewId, children, onRemove, onReset, poppedOut, popoutGeneration = 0, popoutProblem, onPopoutRetry, popoutStopped, popoutCameraMode, popoutCameraOverlays = false, aircraft = null, onAircraft, popoutFlyAircraft = null, aircraftHost = null }: {
  compact?: boolean; viewId?: string; children?: ReactNode; onRemove?: () => void; onReset?: () => void;
  poppedOut?: boolean;
  popoutGeneration?: number;
  popoutProblem?: string | null; onPopoutRetry?: () => void; popoutStopped?: boolean;
  popoutCameraMode?: CameraMode | null;
  popoutCameraOverlays?: boolean;
  aircraft?: AircraftChoice | null; onAircraft?: (aircraft: AircraftChoice | null) => void; popoutFlyAircraft?: FlyAircraftStatus | null;
  aircraftHost?: HTMLElement | null;
  source: DashboardSource | null; client: DashboardClient;
  metadata: ReplayMetadata | null; branchId: string | null; onBranch: (id: string) => void;
  onChooseHistory?: (segmentId: string, parentId: string | null) => Promise<void>;
  options: MapOptions; onOptions: (options: MapOptions) => void;
}) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<(PlainFailure & { retry?: () => void }) | null>(null);
  const [saved, setSaved] = useState<SavedExport | null>(null);
  const { openMapVideoExport } = useExportVideo();
  const branch = metadata?.branches.find(item => item.id === branchId);

  const failed = (sentence: string, cause: unknown, retry?: () => void) => setError({ ...actionFailure(sentence, errorMessage(cause)), retry });
  const run = async (sentence: string, action: () => Promise<void>) => {
    setBusy(true); setError(null);
    try { await action(); menu.close(); }
    catch (cause) { failed(sentence, cause, () => void run(sentence, action)); }
    finally { setBusy(false); }
  };
  const exportImage = async (view?: string) => {
    menu.close();
    setBusy(true); setError(null); setSaved(null);
    try { await afterPaint(); const answer = await exportMap(view); setSaved(answer); if (answer?.warning) menu.popup.current?.showPopover(); }
    catch (cause) { failed(view === 'popout' ? 'Couldn’t export the pop-out map.' : 'Couldn’t export the map.', cause, () => void exportImage(view)); menu.popup.current?.showPopover(); }
    finally { setBusy(false); }
  };
  const camera = (mode: CameraMode, overlays: boolean) => {
    setError(null);
    void setPopoutCameraMode(popoutGeneration, mode, overlays).catch(cause => failed('Couldn’t change the pop-out camera.', cause, () => camera(mode, overlays)));
  };
  const [dismissed, setDismissed] = useState<string | null>(null);
  const problem = poppedOut && popoutProblem && popoutProblem !== dismissed ? actionFailure(popoutStopped ? 'The pop-out map stopped.' : 'The pop-out map didn’t take the last change.', popoutProblem) : null;
  return <>
    <button ref={trigger} type="button" className={compact ? "icon-button map-controls-trigger is-compact" : "icon-button map-controls-trigger"} data-widget-settings={compact || undefined} popoverTarget={id}
      aria-label="Map settings" data-tooltip="Map settings" aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <Popover menu={menu} id={id} className="map-controls-menu" aria-label="Map settings">
      <PopoverHeading title="Map settings" menu={menu} onDelete={onRemove} onReset={onReset} />
      <MapHistorySection metadata={metadata} branch={branch} onBranch={onBranch} busy={busy}
        onChooseHistory={onChooseHistory && ((segmentId, parent) => void run('Couldn’t change the campaign history.', () => onChooseHistory(segmentId, parent)))} />
      <div className="map-menu-section">
        <button type="button" className="map-menu-action" disabled={!desktopAvailable || !metadata || busy} data-tooltip={desktopAvailable ? 'Save the rendered map as a PNG image' : 'Available in the desktop app'} onClick={() => void exportImage(viewId)}><Icon name="save" />Export map image</button>
        <button type="button" className="map-menu-action" disabled={!desktopAvailable || !metadata || busy} data-tooltip={desktopAvailable ? 'Export a constant-frame-rate MP4 video of this map' : 'Available in the desktop app'} onClick={() => { menu.close(); openMapVideoExport({ viewId, label: viewId ? 'Map widget' : 'Campaign map' }); }}><Icon name="play" />Export map video…</button>
        {saved && <p className="map-menu-note" role="status" data-detail={saved.warning ?? undefined}>{saved.warning ? `${saved.message}, but Windows didn’t confirm it reached the disk.` : saved.message}</p>}
        {problem && <FailureLine className="map-menu-error" message={problem.text} detail={problem.detail}
          onRetry={popoutStopped ? undefined : onPopoutRetry} onDismiss={popoutStopped || onPopoutRetry ? undefined : () => setDismissed(popoutProblem ?? null)} />}
        {poppedOut && popoutStopped && <button type="button" className="map-menu-action" disabled={busy} onClick={() => void run('Couldn’t restart the pop-out map.', () => restartPopout(popoutGeneration))}><Icon name="refresh" />Restart pop-out map</button>}
        {poppedOut && desktopAvailable && popoutCameraMode && <div className="map-option-row"><span>Pop-out camera</span>
          <CameraModeControl value={popoutCameraMode} overlays={popoutCameraOverlays}
            onChange={mode => camera(mode, popoutCameraOverlays)} onOverlaysChange={shown => camera(popoutCameraMode, shown)} /></div>}
        {poppedOut && desktopAvailable && popoutCameraMode === 'fly' && onAircraft && <div className="map-option-row"><span>Pop-out aircraft</span>
          <AircraftPicker client={client} source={source} chosen={aircraft} status={popoutFlyAircraft} onChoose={onAircraft} host={aircraftHost} /></div>}
        {poppedOut !== undefined && desktopAvailable && (poppedOut ? <>
          <button type="button" className="map-menu-action" disabled={busy} data-tooltip="Save the pop-out window’s map as a PNG image" onClick={() => void exportImage('popout')}><Icon name="save" />Export pop-out map</button>
          <button type="button" className="map-menu-action" disabled={busy} data-tooltip="Export an MP4 video of the pop-out window’s map" onClick={() => { menu.close(); openMapVideoExport({ viewId: 'popout', label: 'Pop-out map' }); }}><Icon name="play" />Export pop-out map video…</button>
          <button type="button" className="map-menu-action" disabled={busy} data-tooltip="Move the pop-out map to the next monitor" onClick={() => void run('Couldn’t move the pop-out map.', () => nextPopoutMonitor(popoutGeneration))}><Icon name="monitor" />Move pop-out to next monitor</button>
          <button type="button" className="map-menu-action" disabled={busy} onClick={() => void run('Couldn’t close the pop-out map.', () => closePopout(popoutGeneration))}><Icon name="close" />Close pop-out map</button>
        </> : <button type="button" className="map-menu-action" disabled={!metadata || busy} data-tooltip="Show this map alone in its own window, following this app’s clock and settings" onClick={() => void run('Couldn’t pop out the map.', async () => { await openPopout(); })}><Icon name="monitor" />Pop out map</button>)}
        {error && <FailureLine className="map-menu-error" message={error.text} detail={error.detail} onRetry={error.retry} retryDisabled={busy} />}
      </div>
      <div className="map-menu-section">
        <label className="map-option-row"><span>Fog of war</span><input type="checkbox" role="switch" disabled={!source} checked={options.fog} onChange={event => onOptions({ ...options, fog: event.currentTarget.checked })} /></label>
        {!desktopAvailable && <p className="map-menu-note">Map display options apply in the desktop app.</p>}
      </div>
      {open && <MapAppearance key={source ? `${source.id}:${source.revision}` : "empty"} source={source} client={client} viewId={viewId} />}
      {open && children}
    </Popover>
  </>;
}

export function MapHistorySection({ metadata, branch, onBranch, onChooseHistory, busy = false }: {
  metadata: ReplayMetadata | null;
  branch: ReplayMetadata['branches'][number] | null | undefined;
  onBranch: (id: string) => void;
  onChooseHistory?: (segmentId: string, parentId: string | null) => void;
  busy?: boolean;
}) {
  if (!metadata || !branch) return null;
  const labels = historyLabels(metadata.branches);
  return <div className="map-menu-section map-recording-options">
    {metadata.branches.length > 1 && <div className="map-option-row"><span>History</span><Select label="Campaign history" value={branch.id} onChange={onBranch}
      options={metadata.branches.map(item => ({ value: item.id, label: labels.get(item.id) ?? item.id }))} /></div>}
    <small className="map-branch-range">{formatDateTime(branch.firstHour)}<br />to {formatDateTime(branch.lastHour)}</small>
    {branch.fork && <small className="map-branch-range">Play from {formatDateTime(branch.fork.hour)} on was left behind when the game was loaded from an earlier save.</small>}
    {branch.joins.filter(join => join.evidence === 'unconfirmed' || join.evidence === 'unrecorded' || join.evidence === 'chosen').map(join =>
      <JoinNote key={join.segmentId} join={join} busy={busy} onChoose={onChooseHistory && (parent => onChooseHistory(join.segmentId, parent))} />)}
  </div>;
}

function JoinNote({ join, busy, onChoose }: { join: BranchJoin; busy: boolean; onChoose?: (parentId: string | null) => void }) {
  const at = formatDateTime(join.hour);
  const text = join.evidence === 'unconfirmed' ? `From ${at}: the recording can’t tell which save this play was loaded from, so it goes on from the latest play.`
    : join.evidence === 'unrecorded' ? `Before ${at}: play the recorder didn’t see. This goes on from the latest recorded play.`
    : `From ${at}: goes on from the play you chose.`;
  const choosable = onChoose && (join.choices.length > 1 || join.evidence === 'chosen');
  return <>
    <small className="map-branch-range">{text}</small>
    {choosable && <div className="map-option-row"><span>Goes on from</span><Select label={`Play the ${at} stretch goes on from`} value={join.parentId} disabled={busy}
      onChange={value => onChoose(value === 'automatic' ? null : value)}
      options={[...join.choices.map(choice => ({ value: choice.segmentId, label: `Play of ${formatDateTime(choice.firstHour)} – ${formatDateTime(choice.lastHour)}` })),
        ...join.evidence === 'chosen' ? [{ value: 'automatic', label: 'As the recording decides' }] : []]} /></div>}
  </>;
}

function afterPaint(): Promise<void> {
  return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
}
