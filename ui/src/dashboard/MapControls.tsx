import { Popover, PopoverHeading, usePopover } from '../ui/Popover';
import MapAppearance from './MapAppearance';
import type { DashboardClient, DashboardSource } from './client';
import type { ReactNode } from 'react';
import { useId, useState } from 'react';
import { desktopAvailable, errorMessage, exportMap } from '../api';
import type { MapOptions, ReplayMetadata } from '../api';
import { formatDateTime } from '../analytics/calendar';
import { Icon } from '../Icons';
import './mapControls.css';

export default function MapControls({ source, client, metadata, branchId, onBranch, options, onOptions, compact = false, viewId, children, onRemove, onReset }: {
  compact?: boolean; viewId?: string; children?: ReactNode; onRemove?: () => void; onReset?: () => void;
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
    <button ref={trigger} type="button" className={compact ? "dash-icon-button map-controls-trigger is-compact" : "button quiet map-controls-trigger"} popoverTarget={id}
      aria-label="Map settings" data-tooltip="Map settings" aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <Popover menu={menu} id={id} className="map-controls-menu" aria-label="Map settings">
      <PopoverHeading title="Map settings" menu={menu} onDelete={onRemove} onReset={onReset} />
      <div className="map-menu-section">
        <label className="map-option-row"><span>Fog of war</span><input type="checkbox" role="switch" disabled={!source} checked={options.fog} onChange={event => onOptions({ ...options, fog: event.currentTarget.checked })} /></label>
      </div>
      {open && <MapAppearance key={source ? `${source.id}:${source.revision}` : "empty"} source={source} client={client} viewId={viewId} />}
      {metadata && branch && <div className="map-menu-section map-recording-options">
        {sessions.length > 1 && <label className="map-option-row"><span>Session</span><select aria-label="Recording session" value={branch.sessionId} onChange={event => { const next = metadata.branches.find(item => item.sessionId === event.currentTarget.value); if (next) onBranch(next.id); }}>
          {sessions.map((session, index) => <option key={session} value={session}>Session {index + 1}</option>)}
        </select></label>}
        <label className="map-option-row"><span>Branch</span><select aria-label="Recording branch" value={branch.id} onChange={event => onBranch(event.currentTarget.value)}>
          {metadata.branches.filter(item => item.sessionId === branch.sessionId).map((item, index) => <option key={item.id} value={item.id}>Branch {index + 1}</option>)}
        </select></label>
        <small className="map-branch-range">{formatDateTime(branch.firstHour)}<br />to {formatDateTime(branch.lastHour)}</small>
        {branch.parentId && <small className="map-branch-range">Possible continuation of branch {branch.parentId}. Parentage is unconfirmed.</small>}
      </div>}
      <div className="map-menu-section">
        <button type="button" className="map-menu-action" disabled={!desktopAvailable || !metadata || busy} data-tooltip={desktopAvailable ? 'Save the rendered map as a PNG image' : 'Available with the native map in the desktop app'} onClick={() => void run(() => exportMap(viewId))}><Icon name="save" />Export map image</button>
      </div>
      {open && children}
      {!desktopAvailable && <p className="map-menu-note">Map display options apply in the desktop app.</p>}
      {error && <p className="map-menu-error" role="alert">{error}</p>}
    </Popover>
  </>;
}
