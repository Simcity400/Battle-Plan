import { Popover, PopoverHeading, usePopover } from './Popover';
import { Select } from './Select';
import MapAppearance from './MapAppearance';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import type { ReactNode } from 'react';
import { useId, useState } from 'react';
import { desktopAvailable, errorMessage, exportMap } from '../shell/api';
import { closePopout, nextPopoutMonitor, openPopout, restartPopout } from '../shell/popout';
import type { MapOptions, ReplayMetadata } from '../shell/api';
import { formatDateTime } from '../dashboard/calendar';
import { Icon } from './Icons';
import './mapControls.css';

export default function MapControls({ source, client, metadata, branchId, onBranch, options, onOptions, compact = false, viewId, children, onRemove, onReset, poppedOut, popoutGeneration = 0, popoutProblem, popoutStopped }: {
  compact?: boolean; viewId?: string; children?: ReactNode; onRemove?: () => void; onReset?: () => void;
  /** The campaign map's pop-out window state; widget maps omit it. */
  poppedOut?: boolean;
  /** The generation of the pop-out window shown, which its restart and close name. */
  popoutGeneration?: number;
  /** Why the pop-out shows nothing, and whether its viewer exited and can be restarted. */
  popoutProblem?: string | null; popoutStopped?: boolean;
  source: DashboardSource | null; client: DashboardClient;
  metadata: ReplayMetadata | null; branchId: string | null; onBranch: (id: string) => void;
  options: MapOptions; onOptions: (options: MapOptions) => void;
}) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const branch = metadata?.branches.find(item => item.id === branchId);
  const sessions = [...new Set(metadata?.branches.map(item => item.sessionId) ?? [])];

  const run = async (action: () => Promise<void>) => {
    setBusy(true); setError(null);
    try { await action(); menu.close(); }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setBusy(false); }
  };
  return <>
    <button ref={trigger} type="button" className={compact ? "icon-button map-controls-trigger is-compact" : "icon-button map-controls-trigger"} popoverTarget={id}
      aria-label="Map settings" data-tooltip="Map settings" aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <Popover menu={menu} id={id} className="map-controls-menu" aria-label="Map settings">
      <PopoverHeading title="Map settings" menu={menu} onDelete={onRemove} onReset={onReset} />
      <div className="map-menu-section">
        <label className="map-option-row"><span>Fog of war</span><input type="checkbox" role="switch" disabled={!source} checked={options.fog} onChange={event => onOptions({ ...options, fog: event.currentTarget.checked })} /></label>
      </div>
      {open && <MapAppearance key={source ? `${source.id}:${source.revision}` : "empty"} source={source} client={client} viewId={viewId} />}
      {metadata && branch && <div className="map-menu-section map-recording-options">
        {sessions.length > 1 && <div className="map-option-row"><span>Session</span><Select label="Recording session" value={branch.sessionId}
          onChange={session => { const next = metadata.branches.find(item => item.sessionId === session); if (next) onBranch(next.id); }}
          options={sessions.map((session, index) => ({ value: session, label: `Session ${index + 1}` }))} /></div>}
        <div className="map-option-row"><span>Branch</span><Select label="Recording branch" value={branch.id} onChange={onBranch}
          options={metadata.branches.filter(item => item.sessionId === branch.sessionId).map((item, index) => ({ value: item.id, label: `Branch ${index + 1}` }))} /></div>
        <small className="map-branch-range">{formatDateTime(branch.firstHour)}<br />to {formatDateTime(branch.lastHour)}</small>
        {branch.parentId && <small className="map-branch-range">Possible continuation of branch {branch.parentId}. Parentage is unconfirmed.</small>}
      </div>}
      <div className="map-menu-section">
        <button type="button" className="map-menu-action" disabled={!desktopAvailable || !metadata || busy} data-tooltip={desktopAvailable ? 'Save the rendered map as a PNG image' : 'Available with the native map in the desktop app'} onClick={() => void run(() => exportMap(viewId))}><Icon name="save" />Export map image</button>
        {poppedOut && popoutProblem && <p className="map-menu-error" role="alert">{popoutProblem}</p>}
        {poppedOut && popoutStopped && <button type="button" className="map-menu-action" disabled={busy} onClick={() => void run(() => restartPopout(popoutGeneration))}><Icon name="refresh" />Restart pop-out map</button>}
        {poppedOut !== undefined && desktopAvailable && (poppedOut ? <>
          <button type="button" className="map-menu-action" disabled={busy} data-tooltip="Save the pop-out window's map as a PNG image" onClick={() => void run(() => exportMap('popout'))}><Icon name="save" />Export pop-out map</button>
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
