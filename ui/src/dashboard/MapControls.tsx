import MapAppearance from './MapAppearance';
import type { DashboardClient, DashboardSource } from './client';
import type { ReactNode } from 'react';
import { useId, useLayoutEffect, useRef, useState } from 'react';
import { desktopAvailable, errorMessage, exportMap } from '../api';
import type { MapOptions, ReplayMetadata } from '../api';
import { formatDateTime } from '../analytics/calendar';
import { Icon } from '../Icons';
import './mapControls.css';

export default function MapControls({ source, client, metadata, branchId, onBranch, options, onOptions, compact = false, viewId, children }: {
  compact?: boolean; viewId?: string; children?: ReactNode;
  source: DashboardSource | null; client: DashboardClient;
  metadata: ReplayMetadata | null; branchId: string | null; onBranch: (id: string) => void;
  options: MapOptions; onOptions: (options: MapOptions) => void;
}) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const branch = metadata?.branches.find(item => item.id === branchId);
  const sessions = [...new Set(metadata?.branches.map(item => item.sessionId) ?? [])];
  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      if (!trigger.current || !popup.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      const menu = popup.current.getBoundingClientRect();
      popup.current.style.left = `${Math.max(8, Math.min(anchor.right - menu.width, window.innerWidth - menu.width - 8))}px`;
      popup.current.style.top = `${Math.max(8, Math.min(anchor.bottom + 8, window.innerHeight - menu.height - 8))}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    if (popup.current) observer.observe(popup.current);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => { observer.disconnect(); window.removeEventListener('resize', position); window.removeEventListener('scroll', position, true); };
  }, [open]);
  const run = async (action: () => Promise<void>) => {
    setBusy(true); setError(null);
    try { await action(); popup.current?.hidePopover(); trigger.current?.focus(); }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setBusy(false); }
  };
  return <>
    <button ref={trigger} type="button" className={compact ? "dash-icon-button map-controls-trigger is-compact" : "button quiet map-controls-trigger"} popoverTarget={id}
      aria-label="Map settings" data-tooltip="Map settings" aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <div ref={popup} id={id} popover="auto" className="map-controls-menu" role="dialog" aria-label="Map settings"
      onToggle={event => { setOpen(event.newState === 'open'); }}
      onKeyDown={event => { if (event.key === 'Escape') { popup.current?.hidePopover(); trigger.current?.focus(); } }}>
      <div className="map-menu-heading"><strong>Map settings</strong><button type="button" className="dash-icon-button" aria-label="Close map settings" onClick={() => { popup.current?.hidePopover(); trigger.current?.focus(); }}><Icon name="close" /></button></div>
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
    </div>
  </>;
}
