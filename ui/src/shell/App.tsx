import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import {
  desktopAvailable,
  errorMessage,
  openReplay,
  selectNativeView,
  mapAction,
} from './api';
import { Icon } from '../components/Icons';
import type { IconName } from '../components/Icons';
import BrandMark from '../components/BrandMark';
import { useDesktopState, useMapSurface } from './useNativeViewer';
import { nativeDashboardClient } from '../dashboard/nativeClient';
import { browserDashboardClient } from '../dashboard/browserClient';
import Settings, { SettingsNav, settingsSections } from './Settings';
import type { SettingsSection, SettingsTarget } from './Settings';
import { useAppearance } from './appearance/useAppearance';
import Backdrop from './appearance/Backdrop';
import './shell.css';
import { useReplayLibrary } from '../dashboard/replayLibrary';
import { contentRevision } from '../dashboard/contentRevision';
import type { ContentFolders, ReplayMetadata } from './api';
import { hasMapOverlay, isMapOverlay } from './mapOverlays';
import { useDiagnosticsShortcut } from './diagnosticsSetting';
import { RecordingNotice } from './recordingNotice';

const browserRecordings = import.meta.env.DEV && !desktopAvailable;

const Dashboard = lazy(() => import('../dashboard/Dashboard'));
type Tab = 'map' | 'analytics' | 'settings';
const views: { tab: Tab; icon: IconName; label: string }[] = [
  { tab: 'analytics', icon: 'grid', label: 'Overview' },
  { tab: 'map', icon: 'map', label: 'Campaign map' },
];
const SIDEBAR_KEY = 'hoi4-replay.sidebar.v1';
const SIDEBAR_WIDTH_KEY = 'hoi4-replay.sidebar-width.v1';
const SIDEBAR_WIDTH = { initial: 232, min: 200, max: 400, mainMin: 640 } as const;
const clampWidth = (width: number) => Math.round(Math.min(SIDEBAR_WIDTH.max, window.innerWidth - SIDEBAR_WIDTH.mainMin, Math.max(SIDEBAR_WIDTH.min, width)));

/** The sidebar is shown or hidden (Ctrl+B) and resized by dragging its edge, as in T3 Code. */
function useSidebarState() {
  const [hidden, setHidden] = useState(() => { try { return localStorage.getItem(SIDEBAR_KEY) === 'hidden'; } catch { return false; } });
  const [width, setWidth] = useState(() => {
    try { const saved = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY)); return Number.isFinite(saved) && saved > 0 ? clampWidth(saved) : SIDEBAR_WIDTH.initial; }
    catch { return SIDEBAR_WIDTH.initial; }
  });
  const [resizing, setResizing] = useState(false);
  const saveWidth = (next: number) => { try { localStorage.setItem(SIDEBAR_WIDTH_KEY, String(next)); } catch { /* Session-only preference. */ } };
  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const handle = event.currentTarget;
    handle.setPointerCapture(event.pointerId);
    const start = event.clientX, from = width;
    let latest = from;
    setResizing(true);
    const move = (moved: PointerEvent) => { latest = clampWidth(from + moved.clientX - start); setWidth(latest); };
    const end = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', end);
      handle.removeEventListener('pointercancel', end);
      setResizing(false);
      saveWidth(latest);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
  };
  const resetWidth = () => { setWidth(SIDEBAR_WIDTH.initial); try { localStorage.removeItem(SIDEBAR_WIDTH_KEY); } catch { /* Session-only preference. */ } };
  /** Arrow keys step the width (Shift for larger steps); Home and End jump to its limits. */
  const resizeByKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 48 : 16;
    const next = event.key === 'ArrowLeft' ? width - step : event.key === 'ArrowRight' ? width + step
      : event.key === 'Home' ? SIDEBAR_WIDTH.min : event.key === 'End' ? SIDEBAR_WIDTH.max : null;
    if (next === null) return;
    event.preventDefault();
    const clamped = clampWidth(next);
    setWidth(clamped);
    saveWidth(clamped);
  };
  const toggle = useCallback(() => setHidden(value => {
    try { localStorage.setItem(SIDEBAR_KEY, value ? 'shown' : 'hidden'); } catch { /* Session-only preference. */ }
    return !value;
  }), []);
  useEffect(() => {
    const key = (event: globalThis.KeyboardEvent) => {
      if (event.defaultPrevented || !(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey || event.key.toLowerCase() !== 'b') return;
      // Text fields keep their own Ctrl+B.
      const target = event.target as HTMLElement | null;
      if (target?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      event.preventDefault();
      toggle();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [toggle]);
  return { hidden, toggle, width, resizing, startResize, resetWidth, resizeByKey };
}

/** Where the app's update control will live. No update service exists yet, so a check only reports that. */
function UpdateButton() {
  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState(false);
  const label = checking ? 'Checking for updates…' : checked ? 'Automatic updates are not available in this build yet' : 'Check for updates';
  return <button type="button" className={`update-button${checking ? ' is-checking' : ''}`} aria-label={label} data-tooltip={label}
    onClick={() => { if (checking) return; setChecking(true); setTimeout(() => { setChecking(false); setChecked(true); }, 900); }}>
    <Icon name="refresh-cw" />
  </button>;
}

function MapState({ icon = 'map', eyebrow, title, children, busy = false, alert = false }: { icon?: IconName; eyebrow: string; title: string; children?: ReactNode; busy?: boolean; alert?: boolean }) {
  return <div className={`map-state${alert ? ' is-error' : ''}`} role={alert ? 'alert' : 'status'}>
    <div className="map-state-icon">{busy ? <span className="spinner" /> : <Icon name={icon} />}</div>
    <span className="eyebrow">{eyebrow}</span>
    <h1>{title}</h1>
    {children}
  </div>;
}

export default function App() {
  const appearance = useAppearance();
  const sidebar = useSidebarState();
  useDiagnosticsShortcut();
  const [settingsSection, setSettingsSection] = useState<SettingsSection>('folders');
  const [settingsTarget, setSettingsTarget] = useState<SettingsTarget | null>(null);
  const lastView = useRef<Tab>(desktopAvailable ? 'map' : 'analytics');
  const clearSettingsTarget = useCallback(() => setSettingsTarget(null), []);
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
  useEffect(() => { contentRevision(client).observe(folders); }, [client, folders?.game, folders?.user, folders?.contentReady]); // eslint-disable-line react-hooks/exhaustive-deps
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
      if (current) { setSettingsSection('folders'); setTab('settings'); }
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
      if (next !== 'settings') lastView.current = next;
      setTab(next);
    } catch (cause: unknown) {
      setActionError(errorMessage(cause));
    } finally {
      setSwitching(false);
    }
  }

  const nativeReady = native.state?.ready && native.state.running;

  const openSettings = (section: SettingsSection, target: SettingsTarget | null = null) => {
    setSettingsSection(section);
    setSettingsTarget(target);
    if (tab !== 'settings') void changeTab('settings');
  };
  const sectionLabel = settingsSections.find(item => item.id === settingsSection)?.label;

  return (
    <div className={`app-shell${sidebar.resizing ? ' is-resizing' : ''}`} data-sidebar={sidebar.hidden ? 'hidden' : 'shown'} data-tab={tab}
      style={{ '--sidebar-width': sidebar.hidden ? '0px' : `${sidebar.width}px` } as CSSProperties}>
      <Backdrop kind={appearance.settings.backdrop.kind} image={appearance.backdropImage} />
      <aside className="sidebar" aria-label="Battle Plan navigation" inert={sidebar.hidden}>
        <div className="sidebar-header">
          <button type="button" className="icon-button sidebar-toggle" onClick={sidebar.toggle} aria-label="Hide sidebar" data-tooltip="Hide sidebar · Ctrl+B"><Icon name="sidebar" /></button>
          <BrandMark size={20} /><span className="sidebar-wordmark"><strong>the game</strong> Replay</span>
        </div>
        <div className="sidebar-body">
        {tab === 'settings'
          ? <SettingsNav section={settingsSection} onBack={() => void changeTab(lastView.current)} onSection={section => openSettings(section)}
            onTarget={target => openSettings(target.section, target)} disabled={switching} />
          : <nav className="sidebar-menu" aria-label="Views">
            {views.map(view => <button key={view.tab} id={`tab-${view.tab}`} type="button" className="sidebar-row" aria-current={tab === view.tab ? 'page' : undefined}
              disabled={switching} onClick={() => void changeTab(view.tab)}><Icon name={view.icon} /><span>{view.label}</span></button>)}
          </nav>}
        <div className="sidebar-campaign-host" ref={setCampaignHost} hidden={tab === 'settings'} />
        <div className="sidebar-footer">
          {tab !== 'settings' && <button id="tab-settings" type="button" className="icon-button sidebar-utility" disabled={switching} aria-label="Settings" data-tooltip="Settings"
            onClick={() => void changeTab('settings')}><Icon name="settings" /></button>}
          <UpdateButton />
        </div>
        </div>
        {!sidebar.hidden && <div className="sidebar-rail" role="separator" aria-orientation="vertical" aria-label="Resize sidebar" data-tooltip="Drag to resize · double-click to reset"
          tabIndex={0} aria-valuenow={sidebar.width} aria-valuemin={SIDEBAR_WIDTH.min} aria-valuemax={clampWidth(SIDEBAR_WIDTH.max)}
          onPointerDown={sidebar.startResize} onDoubleClick={sidebar.resetWidth} onKeyDown={sidebar.resizeByKey} />}
      </aside>
      <header className="shell-header">
        {sidebar.hidden && <button type="button" className="icon-button sidebar-toggle" onClick={sidebar.toggle} aria-label="Show sidebar" data-tooltip="Show sidebar · Ctrl+B"><Icon name="sidebar" /></button>}
        <div className="shell-campaign-controls" ref={setPlaybackHost} hidden={tab === 'settings'} />
        {tab === 'settings' && <nav className="shell-location" aria-label="Location"><span>Settings</span><Icon name="chevron-right" /><strong aria-current="page">{sectionLabel}</strong></nav>}
        {tab !== 'settings' && <RecordingNotice partial={metadata?.partial} />}
      </header>

      <main className="workspace" ref={content}>
        <section id="panel-settings" aria-label="Settings" tabIndex={-1} className="settings-panel" hidden={tab !== 'settings'}>
          {tab === 'settings' && <Settings section={settingsSection} target={settingsTarget} onTargetShown={clearSettingsTarget} folders={folders} folderError={folderError} onFoldersChange={value => { setBrowserFolders(value); setFolderError(null); }} native={native.state} metadata={metadata} library={replayLibrary} client={client} appearance={appearance} />}
        </section>
        <section id="panel-map" aria-labelledby="tab-map" tabIndex={-1}
          className="map-panel" hidden={tab !== 'map'}>
          <div className="map-placeholder">
            {!desktopAvailable ? (
              <MapState icon="globe" eyebrow="Desktop feature" title="The map lives in the desktop app">
                <p>Open Battle Plan on your desktop to load a recording and explore its native campaign map.</p>
              </MapState>
            ) : mapError ? (
              <MapState icon="alert" eyebrow="Map unavailable" title="The map could not be prepared" alert>
                <p className="error-copy">{mapError}</p>
                <div className="button-row centered">
                  <button type="button" className="button primary" onClick={() => {
                    setActionError(null);
                    setRetry((value) => value + 1);
                  }}><Icon name="refresh" />Retry map</button>
                  <button type="button" className="button" disabled={opening} onClick={() => void openRecording()}>
                    <Icon name="folder-open" />Open recording
                  </button>
                </div>
              </MapState>
            ) : !nativeReady ? (
              <MapState eyebrow="Native viewer" title="Preparing the map" busy>
                <p>Your map and playback controls will appear here.</p>
              </MapState>
            ) : (
              <MapState icon={path ? 'map' : 'folder-open'} eyebrow="Battle Plan" title={path ? 'Opening your recording' : 'Open a recording'} busy={!!path}>
                <p>{path ? 'The native viewer is preparing the recorded map.' : 'Choose a campaign from your replay folder to explore it hour by hour.'}</p>
                {!path && <button type="button" className="button primary" disabled={opening} onClick={() => void openRecording()}>
                  <Icon name="folder-open" />Open recording
                </button>}
              </MapState>
            )}
          </div>
        </section>

        <section id="panel-analytics" aria-labelledby="tab-analytics" tabIndex={-1}
          className="analytics-panel" hidden={tab !== 'analytics'}>
          <Suspense fallback={<div className="panel-loading" role="status"><span className="spinner" />Loading overview…</div>}>
            <Dashboard path={path} client={client} library={replayLibrary} onPrimaryPath={setDefaultPath} campaignHost={campaignHost} onOpenReady={registerOpen} active={tab === 'analytics'} mapActive={tab === 'map'} mapSwitching={switching} mapReady={!!nativeReady} onMetadata={setMetadata} onMapError={setActionError} playbackHost={playbackHost} />
          </Suspense>
        </section>
      </main>
    </div>
  );
}
