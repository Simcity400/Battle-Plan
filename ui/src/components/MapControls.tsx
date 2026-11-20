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
import type { CameraMode, FlyAircraftStatus } from '../generated/dashboard';
import type { MapOptions, ReplayMetadata } from '../shell/api';
import { formatDateTime } from '../dashboard/calendar';
import { Icon } from './Icons';
import type { BranchJoin } from '../generated/dashboard';
import { historyLabels } from '../shell/diagnosticsData';
import './mapControls.css';

export default function MapControls({ source, client, metadata, branchId, onBranch, onChooseHistory, options, onOptions, compact = false, viewId, children, onRemove, onReset, poppedOut, popoutGeneration = 0, popoutProblem, popoutStopped, popoutCameraMode, popoutCameraOverlays = false, aircraft = null, onAircraft, popoutFlyAircraft = null, aircraftHost = null }: {
  compact?: boolean; viewId?: string; children?: ReactNode; onRemove?: () => void; onReset?: () => void;
  /** The campaign map's pop-out window state; widget maps omit it. */
  poppedOut?: boolean;
  /** The generation of the pop-out window shown, which its restart and close name. */
  popoutGeneration?: number;
  /** Why the pop-out shows nothing, and whether its viewer exited and can be restarted. */
  popoutProblem?: string | null; popoutStopped?: boolean;
  /** The pop-out map's camera mode, as its viewer reports it while it runs. */
  popoutCameraMode?: CameraMode | null;
  popoutCameraOverlays?: boolean;
  /** The campaign's aircraft for Fly mode, how to choose another, and what the pop-out's map
   * answered for it; the pop-out offers the choice while its camera flies. */
  aircraft?: string | null; onAircraft?: (aircraft: string | null) => void; popoutFlyAircraft?: FlyAircraftStatus | null;
  /** Where the aircraft list docks, beside the workspace. */
  aircraftHost?: HTMLElement | null;
  source: DashboardSource | null; client: DashboardClient;
  metadata: ReplayMetadata | null; branchId: string | null; onBranch: (id: string) => void;
  /** Joins a recorded stretch of play to the earlier one chosen for it, or (null) as the recording decides. */
  onChooseHistory?: (segmentId: string, parentId: string | null) => Promise<void>;
  options: MapOptions; onOptions: (options: MapOptions) => void;
}) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const branch = metadata?.branches.find(item => item.id === branchId);
  const labels = historyLabels(metadata?.branches ?? []);

  const run = async (action: () => Promise<void>) => {
    setBusy(true); setError(null);
    try { await action(); menu.close(); }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setBusy(false); }
  };
  // The open menu hides the native map, which refuses an export: it closes, and the map shows
  // again, before the export is asked for. A refusal opens the menu again to say why.
  const exportImage = async (view?: string) => {
    menu.close();
    setBusy(true); setError(null);
    try { await afterPaint(); await exportMap(view); }
    catch (cause) { setError(errorMessage(cause)); menu.popup.current?.showPopover(); }
    finally { setBusy(false); }
  };
  return <>
    <button ref={trigger} type="button" className={compact ? "icon-button map-controls-trigger is-compact" : "icon-button map-controls-trigger"} data-widget-settings={compact || undefined} popoverTarget={id}
      aria-label="Map settings" data-tooltip="Map settings" aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <Popover menu={menu} id={id} className="map-controls-menu" aria-label="Map settings">
      <PopoverHeading title="Map settings" menu={menu} onDelete={onRemove} onReset={onReset} />
      <div className="map-menu-section">
        <label className="map-option-row"><span>Fog of war</span><input type="checkbox" role="switch" disabled={!source} checked={options.fog} onChange={event => onOptions({ ...options, fog: event.currentTarget.checked })} /></label>
      </div>
      {open && <MapAppearance key={source ? `${source.id}:${source.revision}` : "empty"} source={source} client={client} viewId={viewId} />}
      {metadata && branch && <div className="map-menu-section map-recording-options">
        {metadata.branches.length > 1 && <div className="map-option-row"><span>History</span><Select label="Campaign history" value={branch.id} onChange={onBranch}
          options={metadata.branches.map(item => ({ value: item.id, label: labels.get(item.id) ?? item.id }))} /></div>}
        <small className="map-branch-range">{formatDateTime(branch.firstHour)}<br />to {formatDateTime(branch.lastHour)}</small>
        {branch.fork && <small className="map-branch-range">Play from {formatDateTime(branch.fork.hour)} on was left behind when the game was loaded from an earlier save.</small>}
        {branch.joins.filter(join => join.evidence === 'unconfirmed' || join.evidence === 'unrecorded' || join.evidence === 'chosen').map(join =>
          <JoinNote key={join.segmentId} join={join} busy={busy} onChoose={onChooseHistory && (parent => void run(() => onChooseHistory(join.segmentId, parent)))} />)}
      </div>}
      <div className="map-menu-section">
        <button type="button" className="map-menu-action" disabled={!desktopAvailable || !metadata || busy} data-tooltip={desktopAvailable ? 'Save the rendered map as a PNG image' : 'Available with the native map in the desktop app'} onClick={() => void exportImage(viewId)}><Icon name="save" />Export map image</button>
        {poppedOut && popoutProblem && <p className="map-menu-error" role="alert">{popoutProblem}</p>}
        {poppedOut && popoutStopped && <button type="button" className="map-menu-action" disabled={busy} onClick={() => void run(() => restartPopout(popoutGeneration))}><Icon name="refresh" />Restart pop-out map</button>}
        {poppedOut && desktopAvailable && popoutCameraMode && <div className="map-option-row"><span>Pop-out camera</span>
          <CameraModeControl value={popoutCameraMode} overlays={popoutCameraOverlays}
            onChange={mode => void setPopoutCameraMode(popoutGeneration, mode, popoutCameraOverlays).catch(cause => setError(errorMessage(cause)))}
            onOverlaysChange={shown => void setPopoutCameraMode(popoutGeneration, popoutCameraMode, shown).catch(cause => setError(errorMessage(cause)))} /></div>}
        {poppedOut && desktopAvailable && popoutCameraMode === 'fly' && onAircraft && <div className="map-option-row"><span>Pop-out aircraft</span>
          <AircraftPicker client={client} source={source} chosen={aircraft} status={popoutFlyAircraft} onChoose={onAircraft} host={aircraftHost} /></div>}
        {poppedOut !== undefined && desktopAvailable && (poppedOut ? <>
          <button type="button" className="map-menu-action" disabled={busy} data-tooltip="Save the pop-out window's map as a PNG image" onClick={() => void exportImage('popout')}><Icon name="save" />Export pop-out map</button>
          <button type="button" className="map-menu-action" disabled={busy} data-tooltip="Move the pop-out map to the next monitor" onClick={() => void run(() => nextPopoutMonitor(popoutGeneration))}><Icon name="monitor" />Move pop-out to next monitor</button>
          <button type="button" className="map-menu-action" disabled={busy} onClick={() => void run(() => closePopout(popoutGeneration))}><Icon name="close" />Close pop-out map</button>
        </> : <button type="button" className="map-menu-action" disabled={!metadata || busy} data-tooltip="Show this map alone in its own window, following this app's clock and settings" onClick={() => void run(async () => { await openPopout(); })}><Icon name="monitor" />Pop out map</button>)}
      </div>
      {open && children}
      {!desktopAvailable && <p className="map-menu-note">Map display options apply in the desktop app.</p>}
      {error && <p className="map-menu-error" role="alert">{error}</p>}
    </Popover>
  </>;
}

/** A join the recording can't prove, or that was chosen: what it means, and the earlier play it may go on from. */
function JoinNote({ join, busy, onChoose }: { join: BranchJoin; busy: boolean; onChoose?: (parentId: string | null) => void }) {
  const at = formatDateTime(join.hour);
  const text = join.evidence === 'unconfirmed' ? `From ${at}: the recording can't tell which save this play was loaded from, so it goes on from the latest play.`
    : join.evidence === 'unrecorded' ? `Before ${at}: play the recorder didn't see. This goes on from the latest recorded play.`
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

/** Resolves once the frame after the next has painted: layout effects and the map's placement have run. */
function afterPaint(): Promise<void> {
  return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
}
