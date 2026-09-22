import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import {
  desktopAvailable,
  errorMessage,
  openReplay,
  selectNativeView,
  mapAction,
} from './api';
import { Icon } from '../components/Icons';
import { useDesktopState, useMapSurface } from './useNativeViewer';
import { nativeDashboardClient } from '../dashboard/nativeClient';
import { browserDashboardClient } from '../dashboard/browserClient';
import Settings from './Settings';
import { useTheme } from './theme';
import { useReplayLibrary } from '../dashboard/replayLibrary';
import type { ContentFolders, ReplayMetadata } from './api';
import { hasMapOverlay, isMapOverlay } from './mapOverlays';

const browserRecordings = import.meta.env.DEV && !desktopAvailable;

const Dashboard = lazy(() => import('../dashboard/Dashboard'));
type Tab = 'map' | 'analytics' | 'settings';
const tabs: Tab[] = ['analytics', 'map', 'settings'];

export default function App() {
  const appearance = useTheme();
  const client = browserRecordings ? browserDashboardClient : nativeDashboardClient;
  const replayLibrary = useReplayLibrary(client);
  const defaultChosen = useRef(false);
  const [tab, setTab] = useState<Tab>(desktopAvailable ? 'map' : 'analytics');
  const [playbackHost, setPlaybackHost] = useState<HTMLDivElement | null>(null);
  const [campaignHost, setCampaignHost] = useState<HTMLDivElement | null>(null);
  const [metadata, setMetadata] = useState<ReplayMetadata | null>(null);
  const [browserFolders, setBrowserFolders] = useState<ContentFolders | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  const setupChecked = useRef(false);
  const openCampaign = useRef<(() => void) | null>(null);
  const registerOpen = useCallback((open: (() => void) | null) => { openCampaign.current = open; }, []);
  const [retry, setRetry] = useState(0);
  const [path, setPath] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [popoverOpen, setPopoverOpen] = useState(false);
  useEffect(() => {
    const opening = (event: Event) => { if ('newState' in event && event.newState === 'open' && isMapOverlay(event.target)) setPopoverOpen(true); };
    const toggled = () => setPopoverOpen(hasMapOverlay());
    document.addEventListener('beforetoggle', opening, true);
    document.addEventListener('toggle', toggled, true);
    return () => { document.removeEventListener('beforetoggle', opening, true); document.removeEventListener('toggle', toggled, true); };
  }, []);
  const pendingPath = useRef<string | null>(null);
  const defaultRequest = useRef(0);
  const lastNativePath = useRef<string | null>(null);
  const content = useRef<HTMLElement>(null);
  const native = useDesktopState(retry);
  const folders = desktopAvailable ? native.state?.mapDetails ?? null : browserFolders;
  useEffect(() => {
    if (!client.contentFolders) return;
    const controller = new AbortController();
    void client.contentFolders(controller.signal).then(value => { if (!controller.signal.aborted) setBrowserFolders(value); })
      .catch(cause => { if (!controller.signal.aborted) setFolderError(errorMessage(cause)); });
    return () => controller.abort();
  }, [client]);
  useEffect(() => {
    if (!desktopAvailable || !native.state?.ready) return;
    let current = true;
    void mapAction({ kind: 'inspect' }).catch(cause => { if (current) setFolderError(errorMessage(cause)); });
    return () => { current = false; };
  }, [native.state?.ready]);
  useEffect(() => {
    if (!folders || setupChecked.current) return;
    setupChecked.current = true;
    if (folders.contentReady) return;
    let current = true;
    void selectNativeView(true).then(() => {
      if (current) { setTab('settings'); }
    }).catch(cause => { if (current) setFolderError(errorMessage(cause)); });
    return () => { current = false; };
  }, [folders?.contentReady]);
  // Native load/configuration failures keep their existing controls reachable.
  // Only shell-owned failures need the child hidden for the web recovery panel.
  const shellError = actionError ?? native.error;
  const boundsError = useMapSurface(content, tab === 'map' && !!path && !shellError && !popoverOpen, retry, tab === 'map');
  const nativePath = native.state?.viewer.path ?? null;
  const mapError = actionError ?? boundsError ?? native.error ?? native.state?.error ?? null;

  useEffect(() => {
    const changed = lastNativePath.current !== nativePath;
    lastNativePath.current = nativePath;
    // An initial command-line recording can finish opening after Analytics is
    // selected. Adopt it only while no dashboard recording has been chosen.
    if (path != null) { if (nativePath === pendingPath.current) pendingPath.current = null; return; }
    if (!nativePath || defaultChosen.current) return;
    if (pendingPath.current && nativePath !== pendingPath.current) return;
    const acknowledged = pendingPath.current === nativePath;
    pendingPath.current = null;
    if (changed || acknowledged || path == null) setPath(nativePath);
  }, [nativePath, native.state?.viewer, tab, path]);

  useEffect(() => {
    if (native.state?.error) pendingPath.current = null;
  }, [native.state?.error]);

  function openRecording() { openCampaign.current?.(); }
  const setDefaultPath = useCallback((chosen: string | null) => {
    defaultChosen.current = true;
    setPath(chosen);
    setActionError(null);
    const request = ++defaultRequest.current;
    if (!chosen || !desktopAvailable || tab !== 'map' || (chosen === nativePath && !pendingPath.current)) { setOpening(false); return; }
    pendingPath.current = chosen;
    setOpening(true);
    void openReplay(chosen).catch((cause) => {
      if (request === defaultRequest.current) { pendingPath.current = null; setActionError(errorMessage(cause)); }
    }).finally(() => { if (request === defaultRequest.current) setOpening(false); });
  }, [tab, nativePath]);

  async function changeTab(next: Tab) {
    if (switching || next === tab) return;
    setSwitching(true);
    try {
      await selectNativeView(next !== 'map');
      if (desktopAvailable && next === 'map' && path && path !== nativePath) {
        pendingPath.current = path;
        await openReplay(path);
      }
      setActionError(null);
      setTab(next);
      requestAnimationFrame(() => {
        document.getElementById(`tab-${next}`)?.focus({ preventScroll: true });
      });
    } catch (cause: unknown) {
      setActionError(errorMessage(cause));
    } finally {
      setSwitching(false);
    }
  }

  function moveTab(event: KeyboardEvent<HTMLButtonElement>, current: Tab) {
    const index = tabs.indexOf(current);
    let next: Tab | undefined;
    if (event.key === 'ArrowDown' || event.key === 'ArrowRight') next = tabs[(index + 1) % tabs.length];
    if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') next = tabs[(index + tabs.length - 1) % tabs.length];
    if (event.key === 'Home') next = tabs[0];
    if (event.key === 'End') next = tabs.at(-1);
    if (!next) return;
    event.preventDefault();
    void changeTab(next);
  }

  const nativeReady = native.state?.ready && native.state.running;

  return (
    <div className="app-shell">
      <header className="shell-header">
        <div className="shell-brand"><Icon name="globe" /><strong>Battle Plan</strong></div>
        <div className="shell-campaign-controls" ref={setPlaybackHost} hidden={tab === 'settings'} />
        {tab === 'settings' && <div className="shell-location"><strong>Settings</strong></div>}
      </header>
      <aside className="side-rail" aria-label="Battle Plan navigation">
        <span className="nav-section-label">Workspace</span>
        <div className="view-tabs" role="tablist" aria-label="Views" aria-orientation="vertical">
          {tabs.map((item) => (
            <button key={item} id={`tab-${item}`} type="button" role="tab"
              className="rail-button" aria-selected={tab === item} aria-controls={`panel-${item}`}
              tabIndex={tab === item ? 0 : -1} disabled={switching} onClick={() => void changeTab(item)}
              onKeyDown={(event) => moveTab(event, item)}>
              <Icon name={item === 'map' ? 'map' : item === 'settings' ? 'settings' : 'grid'} />
              <span>{item === 'map' ? 'Campaign map' : item === 'settings' ? 'Settings' : 'Overview'}</span>
            </button>
          ))}
        </div>
        <div className="rail-divider" />
        <div className="sidebar-campaign-host" ref={setCampaignHost} />
      </aside>

      <main className="workspace" ref={content}>
        <section id="panel-settings" role="tabpanel" aria-labelledby="tab-settings" tabIndex={0} className="settings-panel" hidden={tab !== 'settings'}>
          {tab === 'settings' && <Settings folders={folders} folderError={folderError} onFoldersChange={value => { setBrowserFolders(value); setFolderError(null); }} native={native.state} metadata={metadata} library={replayLibrary} client={client} theme={appearance.theme} onChange={appearance.change} notice={appearance.notice} />}
        </section>
        <section id="panel-map" role="tabpanel" aria-labelledby="tab-map" tabIndex={0}
          className="map-panel" hidden={tab !== 'map'}>
          <div className="map-placeholder">
            <div className="placeholder-icon"><Icon name="map" /></div>
            {!desktopAvailable ? (
              <>
                <h1>The map lives in the desktop app</h1>
                <p>Open Battle Plan on your desktop to load a recording and explore its native map.</p>
              </>
            ) : mapError ? (
              <div role="alert">
                <span className="eyebrow">Map unavailable</span>
                <h1>The map could not be prepared</h1>
                <p className="error-copy">{mapError}</p>
                <div className="button-row centered">
                  <button type="button" className="button primary" onClick={() => {
                    setActionError(null);
                    setRetry((value) => value + 1);
                  }}><Icon name="refresh" />Retry map</button>
                  <button type="button" className="button" disabled={opening} onClick={() => void openRecording()}>
                    <Icon name="folder" />Open recording
                  </button>
                </div>
              </div>
            ) : !nativeReady ? (
              <div role="status">
                <span className="eyebrow">Native viewer</span>
                <h1>Preparing the map</h1>
                <p>Your map and playback controls will appear here.</p>
                <span className="spinner" />
              </div>
            ) : (
              <>
                <span className="eyebrow">Battle Plan</span>
                <h1>{path ? 'Opening your recording' : 'Open a recording'}</h1>
                <p>{path ? 'The native viewer is preparing the recorded map.' : 'Choose a campaign from your replay folder to explore it hour by hour.'}</p>
                {!path && <button type="button" className="button primary" disabled={opening} onClick={() => void openRecording()}>
                  <Icon name="folder" />Open recording
                </button>}
              </>
            )}
          </div>
        </section>

        <section id="panel-analytics" role="tabpanel" aria-labelledby="tab-analytics" tabIndex={0}
          className="analytics-panel" hidden={tab !== 'analytics'}>
          <Suspense fallback={<div className="panel-loading" role="status"><span className="spinner" />Loading analytics…</div>}>
            <Dashboard path={path} client={client} library={replayLibrary} onPrimaryPath={setDefaultPath} campaignHost={campaignHost} onOpenReady={registerOpen} active={tab === 'analytics'} mapActive={tab === 'map'} mapSwitching={switching} mapReady={!!nativeReady} onMetadata={setMetadata} onMapError={setActionError} playbackHost={playbackHost} />
          </Suspense>
        </section>
      </main>
    </div>
  );
}
