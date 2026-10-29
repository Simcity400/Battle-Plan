import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import {
  desktopAvailable,
  errorMessage,
  openReplay,
  selectNativeView,
} from './api';
import { Icon } from '../components/Icons';
import type { IconName } from '../components/Icons';
import { loadable } from '../components/loadable';
import { defaultPages, loadPages, savePages } from '../dashboard/pages';
import type { PageEntry, PageId } from '../dashboard/pages';
import { clearLayouts, layoutsAtDefault } from '../dashboard/storage';
import { widgetDefinitions } from '../dashboard/definitions';
import { stableJson } from '../dashboard/model';
import { sameCampaignPath } from '../dashboard/campaigns';
import AnalyticsNav from './AnalyticsNav';
import BrandMark from '../components/BrandMark';
import { useDesktopState, useMapSurface } from './useNativeViewer';
import { nativeDashboardClient } from '../dashboard/nativeClient';
import { browserDashboardClient } from '../dashboard/browserClient';
import type { BranchChoice } from '../dashboard/Dashboard';
import { settingsSections } from './settingsMetadata';
import type { SettingsSection, SettingsTarget } from './settingsMetadata';
import { useAppearance } from './appearance/useAppearance';
import { useSidebarIconMenu } from './appearance/SidebarIconMenu';
import { StyledIcon } from './appearance/StyledIcon';
import { useWidgetIconMenu, WidgetIconsContext } from './appearance/WidgetIconMenu';
import Backdrop from './appearance/Backdrop';
import './shell.css';
import { useReplayLibrary } from '../dashboard/replayLibrary';
import { contentRevision } from '../dashboard/contentRevision';
import type { ContentFolders, ReplayMetadata } from './api';
import { hasMapOverlay, isMapOverlay } from './mapOverlays';
import { useDiagnosticsShortcut } from './diagnosticsSetting';
import { RecordingNotice } from './recordingNotice';

const browserRecordings = import.meta.env.DEV && !desktopAvailable;

// Shell-only changes (sidebar, popovers, native state) leave the dashboard and its header alone.
const Dashboard = memo(loadable(() => import('../dashboard/Dashboard').then(module => module.default),
  () => <div className="panel-loading" role="status"><span className="spinner" />Loading overview…</div>));
const Settings = loadable(() => import('./Settings').then(module => module.default), () => <p role="status">Loading settings…</p>);
const SettingsNav = loadable(() => import('./Settings').then(module => module.SettingsNav));
type Tab = 'map' | 'analytics' | 'settings';
const TAB_NAMES: Record<Tab, string> = { map: 'The campaign map', analytics: 'Analytics', settings: 'Settings' };
const PAGE_KEY = 'hoi4-replay.analytics-page.v1';
const SIDEBAR_KEY = 'hoi4-replay.sidebar.v1';
const SIDEBAR_WIDTH_KEY = 'hoi4-replay.sidebar-width.v1';
const SIDEBAR_WIDTH = { initial: 232, min: 200, max: 400, mainMin: 640 } as const;
// A window too narrow to leave the workspace its minimum still keeps the sidebar at its own.
const clampWidth = (width: number) => Math.round(Math.max(SIDEBAR_WIDTH.min, Math.min(SIDEBAR_WIDTH.max, window.innerWidth - SIDEBAR_WIDTH.mainMin, width)));

/** The sidebar is shown or hidden (Ctrl+B) and resized by dragging its edge, as in T3 Code. */
function useSidebarState() {
  const [hidden, setHidden] = useState(() => { try { return localStorage.getItem(SIDEBAR_KEY) === 'hidden'; } catch { return false; } });
  // The width chosen, shown as wide as the window allows; a window widened again shows it whole.
  const [chosen, setChosen] = useState(() => {
    try { const saved = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY)); return Number.isFinite(saved) && saved > 0 ? saved : SIDEBAR_WIDTH.initial; }
    catch { return SIDEBAR_WIDTH.initial; }
  });
  const [, setWindowWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const resize = () => setWindowWidth(window.innerWidth);
    window.addEventListener('resize', resize);
    return () => window.removeEventListener('resize', resize);
  }, []);
  const width = clampWidth(chosen);
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
    const move = (moved: PointerEvent) => { latest = clampWidth(from + moved.clientX - start); setChosen(latest); };
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
  const resetWidth = () => { setChosen(SIDEBAR_WIDTH.initial); try { localStorage.removeItem(SIDEBAR_WIDTH_KEY); } catch { /* Session-only preference. */ } };
  /** Arrow keys step the width (Shift for larger steps); Home and End jump to its limits. */
  const resizeByKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 48 : 16;
    const next = event.key === 'ArrowLeft' ? width - step : event.key === 'ArrowRight' ? width + step
      : event.key === 'Home' ? SIDEBAR_WIDTH.min : event.key === 'End' ? SIDEBAR_WIDTH.max : null;
    if (next === null) return;
    event.preventDefault();
    const clamped = clampWidth(next);
    setChosen(clamped);
    saveWidth(clamped);
  };
  // A hidden sidebar slides over the workspace while the pointer is on it or a field in it is being typed in.
  // Native listeners: portalled sidebar content does not bubble React events to the aside.
  const [peek, setPeek] = useState(false);
  const aside = useRef<HTMLElement>(null);
  useEffect(() => {
    const element = aside.current;
    if (!hidden || !element) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const open = () => { clearTimeout(timer); setPeek(true); };
    const close = () => {
      clearTimeout(timer);
      timer = setTimeout(() => { if (!element.matches(':hover, :has(:is(input, select, textarea):focus)')) setPeek(false); }, 250);
    };
    element.addEventListener('pointerenter', open);
    element.addEventListener('pointerleave', close);
    element.addEventListener('focusout', close);
    return () => {
      clearTimeout(timer);
      element.removeEventListener('pointerenter', open);
      element.removeEventListener('pointerleave', close);
      element.removeEventListener('focusout', close);
    };
  }, [hidden]);
  const toggle = useCallback(() => {
    setPeek(false);
    setHidden(value => {
      try { localStorage.setItem(SIDEBAR_KEY, value ? 'shown' : 'hidden'); } catch { /* Session-only preference. */ }
      return !value;
    });
  }, []);
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
  return { hidden, peek: hidden && peek, openPeek: () => setPeek(true), aside, toggle, width, resizing, startResize, resetWidth, resizeByKey };
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
  const iconMenu = useSidebarIconMenu(appearance);
  const widgetIconMenu = useWidgetIconMenu(appearance);
  const sidebar = useSidebarState();
  // A manual popover gives sidebar peek the same native-window occlusion events as menus.
  const peekOverlay = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const overlay = peekOverlay.current;
    if (!sidebar.peek || !overlay) return;
    overlay.showPopover();
    return () => overlay.hidePopover();
  }, [sidebar.peek]);
  useDiagnosticsShortcut();
  const [settingsSection, setSettingsSection] = useState<SettingsSection>('folders');
  const [settingsTarget, setSettingsTarget] = useState<SettingsTarget | null>(null);
  const lastView = useRef<Tab>(desktopAvailable ? 'map' : 'analytics');
  const clearSettingsTarget = useCallback(() => setSettingsTarget(null), []);
  const client = browserRecordings ? browserDashboardClient : nativeDashboardClient;
  const replayLibrary = useReplayLibrary(client);
  const defaultChosen = useRef(false);
  const [tab, setTab] = useState<Tab>(desktopAvailable ? 'map' : 'analytics');
  const [pages, setPages] = useState(loadPages);
  const [page, setPage] = useState<PageId>(() => { try { const saved = localStorage.getItem(PAGE_KEY); return pages.find(entry => entry.id === saved)?.id ?? pages[0].id; } catch { return pages[0].id; } });
  const [layoutReset, setLayoutReset] = useState(0);
  const [pageStorageWarning, setPageStorageWarning] = useState<string | null>(null);
  const choosePage = (next: PageId) => { setPage(next); try { localStorage.setItem(PAGE_KEY, next); } catch { /* Session-only preference. */ } };
  const changePages = (next: PageEntry[]) => { setPages(next); setPageStorageWarning(savePages(next)); };
  const deletePage = (id: PageId) => {
    const index = pages.findIndex(entry => entry.id === id);
    const next = pages.filter(entry => entry.id !== id);
    if (index < 0 || !next.length) return;
    changePages(next);
    clearLayouts(id);
    if (page === id) choosePage(next[Math.min(index, next.length - 1)].id);
  };
  /** Restores the built-in pages, removes added ones and resets every page's layouts. */
  const resetPages = () => {
    for (const entry of [...pages, ...defaultPages()]) clearLayouts(entry.id);
    const next = defaultPages();
    changePages(next);
    choosePage(next[0].id);
    setLayoutReset(count => count + 1);
  };
  // Default layouts are built from the widgets' plain definitions, which load with the
  // dashboard; Settings opened first loads them itself, off the startup path.
  const [widgetsLoading, setWidgetsLoading] = useState<'loading' | 'loaded' | 'failed'>('loading');
  const [layoutsUnsaved, setLayoutsUnsaved] = useState(false);
  useEffect(() => {
    if (tab !== 'settings' || widgetDefinitions().length) return;
    import('../widgets/definitions').then(() => setWidgetsLoading('loaded'), () => setWidgetsLoading('failed'));
  }, [tab]);
  /** Null while the default layouts cannot be known yet; undefined when everything is at its default. */
  const pagesReset = (): (() => void) | null | undefined => {
    if (!widgetDefinitions().length) return widgetsLoading === 'failed' ? resetPages : null;
    return !layoutsUnsaved && stableJson(pages) === stableJson(defaultPages()) && pages.every(entry => layoutsAtDefault(entry.id)) ? undefined : resetPages;
  };
  const [playbackHost, setPlaybackHost] = useState<HTMLDivElement | null>(null);
  const [campaignHost, setCampaignHost] = useState<HTMLDivElement | null>(null);
  // Panels docked beside the workspace, such as the aircraft list, narrow the map rather than
  // cover it, so it stays shown while they are open.
  const [sideHost, setSideHost] = useState<HTMLElement | null>(null);
  const [metadata, setMetadata] = useState<ReplayMetadata | null>(null);
  const [branch, setBranch] = useState<BranchChoice | null>(null);
  // The host's folders in the desktop app, the development bridge's in a browser.
  const [folders, setFolders] = useState<ContentFolders | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  const setupChecked = useRef(false);
  const openCampaign = useRef<(() => void) | null>(null);
  const registerOpen = useCallback((open: (() => void) | null) => { openCampaign.current = open; }, []);
  const [retry, setRetry] = useState(0);
  const [path, setPath] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [switching, setSwitching] = useState(false);
  // A view switch that failed, kept whole until a switch succeeds; the view still shown offers it again.
  const [switchFailure, setSwitchFailure] = useState<{ tab: Tab; message: string } | null>(null);
  // Each failed map operation's error, by operation, until that operation succeeds or is no longer wanted.
  const [mapFailures, setMapFailures] = useState<Record<string, string>>({});
  const reportMap = useCallback((operation: string, error: string | null) => setMapFailures(previous => {
    if ((previous[operation] ?? null) === error) return previous;
    const next = { ...previous };
    if (error == null) delete next[operation]; else next[operation] = error;
    return next;
  }), []);
  const actionError = Object.values(mapFailures)[0] ?? null;
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
  useEffect(() => { contentRevision(client).observe(folders); }, [client, folders?.game, folders?.user, folders?.contentReady]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!client.contentFolders) return;
    const controller = new AbortController();
    void client.contentFolders(controller.signal).then(value => { if (!controller.signal.aborted) setFolders(value); })
      .catch(cause => { if (!controller.signal.aborted) setFolderError(errorMessage(cause)); });
    return () => controller.abort();
  }, [client]);
  useEffect(() => {
    if (!folders || setupChecked.current) return;
    setupChecked.current = true;
    if (folders.contentReady) return;
    let current = true;
    void selectNativeView(true).then(() => {
      if (current) { setSettingsSection('folders'); setSwitchFailure(null); setTab('settings'); }
    }).catch(cause => { if (current) setFolderError(errorMessage(cause)); });
    return () => { current = false; };
  }, [folders?.contentReady]);
  // Native load/configuration failures keep their existing controls reachable.
  // Only shell-owned failures need the child hidden for the web recovery panel.
  const shellError = actionError ?? native.error;
  const boundsError = useMapSurface(content, tab === 'map' && !!path && !shellError && !popoverOpen && !sidebar.peek, retry, tab === 'map');
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
  /** Opens the dashboard's recording on the map; a failure stays shown until a later open succeeds. */
  const openDefault = useCallback((chosen: string | null) => {
    const request = ++defaultRequest.current;
    if (!chosen || !desktopAvailable || tab !== 'map' || (chosen === nativePath && !pendingPath.current)) { setOpening(false); reportMap('open', null); return; }
    pendingPath.current = chosen;
    setOpening(true);
    void openReplay(chosen).then(() => { if (request === defaultRequest.current) reportMap('open', null); }, (cause) => {
      if (request === defaultRequest.current) { pendingPath.current = null; reportMap('open', errorMessage(cause)); }
    }).finally(() => { if (request === defaultRequest.current) setOpening(false); });
  }, [tab, nativePath, reportMap]);
  const setDefaultPath = useCallback((chosen: string | null) => {
    defaultChosen.current = true;
    setPath(chosen);
    reportMap('open', null);
    openDefault(chosen);
  }, [openDefault, reportMap]);

  /** Shows `next`: the native view it needs and, for the map, the dashboard's recording opened on it. */
  async function changeTab(next: Tab) {
    if (switching || next === tab) return;
    setSwitching(true);
    let selected = false;
    try {
      await selectNativeView(next !== 'map');
      selected = true;
      if (desktopAvailable && next === 'map' && path && path !== nativePath) {
        pendingPath.current = path;
        await openReplay(path);
      }
      setSwitchFailure(null);
      // Showing the map opens its recording again, so a failed open goes with any switch.
      reportMap('open', null);
      if (next !== 'settings') lastView.current = next;
      setTab(next);
    } catch (cause: unknown) {
      // Only the open failed: the view still shown takes its native view back, or the map would cover it.
      if (selected) { pendingPath.current = null; await selectNativeView(tab !== 'map').catch(() => {}); }
      setSwitchFailure({ tab: next, message: errorMessage(cause) });
    } finally {
      setSwitching(false);
    }
  }

  /** Sends each failed map operation again; its error stays until that operation succeeds. */
  function retryMap() {
    if (mapFailures.open != null) openDefault(path);
    setRetry(value => value + 1);
  }

  const workspaceBackdrop = appearance.settings.workspaceBackdrop;
  const nativeReady = native.state?.ready && native.state.running;

  const openSettings = (section: SettingsSection, target: SettingsTarget | null = null) => {
    setSettingsSection(section);
    setSettingsTarget(target);
    if (tab !== 'settings') void changeTab('settings');
  };
  const sectionLabel = settingsSections.find(item => item.id === settingsSection)?.label;

  return (
    <div className={`app-shell${sidebar.resizing ? ' is-resizing' : ''}`} data-sidebar={sidebar.peek ? 'peek' : sidebar.hidden ? 'hidden' : 'shown'} data-tab={tab}
      style={{ '--sidebar-width': sidebar.hidden ? '0px' : `${sidebar.width}px`, '--sidebar-peek-width': `${sidebar.width}px` } as CSSProperties}>
      <Backdrop backdrop={appearance.settings.backdrop} image={appearance.backdropImage} />
      <div ref={peekOverlay} popover="manual" className="sidebar-peek-overlay" aria-hidden="true" />
      {sidebar.hidden && <div className="sidebar-edge" aria-hidden="true" onPointerEnter={sidebar.openPeek} />}
      <aside ref={sidebar.aside} className="sidebar" aria-label="Battle Plan navigation" inert={sidebar.hidden && !sidebar.peek}
        onContextMenu={iconMenu.onContextMenu} onKeyDown={iconMenu.onKeyDown}>
        <div className="sidebar-header">
          <button type="button" className="icon-button sidebar-toggle" onClick={sidebar.toggle} aria-label={sidebar.hidden ? 'Keep sidebar open' : 'Hide sidebar'}
            data-tooltip={`${sidebar.hidden ? 'Keep sidebar open' : 'Hide sidebar'} · Ctrl+B`}><Icon name="sidebar" /></button>
          <BrandMark size={20} /><span className="sidebar-wordmark"><strong>the game</strong> Replay</span>
        </div>
        <div className="sidebar-body">
        {tab === 'settings'
          ? <SettingsNav section={settingsSection} icons={appearance.settings.sidebarIcons} onBack={() => void changeTab(lastView.current)} onSection={section => openSettings(section)}
            onTarget={target => openSettings(target.section, target)} disabled={switching} />
          : <nav className="sidebar-menu" aria-label="Views">
            <AnalyticsNav pages={pages} page={page} active={tab === 'analytics'} disabled={switching} onPages={changePages} onDelete={deletePage} iconMenu={iconMenu}
              onChoose={id => { choosePage(id); void changeTab('analytics'); }} />
            <button id="tab-map" type="button" className="sidebar-row" data-sidebar-icon="map" aria-current={tab === 'map' ? 'page' : undefined}
              disabled={switching} onClick={() => void changeTab('map')}><StyledIcon icon={iconMenu.icon('map')} fallback="map" colour="var(--sidebar-icon-map)" /><span>Campaign map</span></button>
          </nav>}
        <div className="sidebar-campaign-host" ref={setCampaignHost} hidden={tab === 'settings'} />
        <div className="sidebar-footer">
          {tab !== 'settings' && <button id="tab-settings" type="button" className="icon-button sidebar-utility" data-sidebar-icon="settings" disabled={switching} aria-label="Settings" data-tooltip="Settings"
            onClick={() => void changeTab('settings')}><StyledIcon icon={iconMenu.icon('settings')} fallback="settings" colour="var(--sidebar-icon-settings)" /></button>}
          <UpdateButton />
        </div>
        </div>
        {!sidebar.hidden && <div className="sidebar-rail" role="separator" aria-orientation="vertical" aria-label="Resize sidebar" data-tooltip="Drag to resize · double-click to reset"
          tabIndex={0} aria-valuenow={sidebar.width} aria-valuemin={SIDEBAR_WIDTH.min} aria-valuemax={clampWidth(SIDEBAR_WIDTH.max)}
          onPointerDown={sidebar.startResize} onDoubleClick={sidebar.resetWidth} onKeyDown={sidebar.resizeByKey} />}
      </aside>
      {iconMenu.menu}
      {widgetIconMenu.menu}
      <header className="shell-header">
        <div className="shell-campaign-controls" ref={setPlaybackHost} hidden={tab === 'settings'} />
        {tab === 'settings' && <nav className="shell-location" aria-label="Location"><span>Settings</span><Icon name="chevron-right" /><strong aria-current="page">{sectionLabel}</strong></nav>}
        {tab !== 'settings' && <RecordingNotice partial={metadata?.partial} damaged={metadata?.damaged} path={metadata?.path} />}
        {pageStorageWarning && <div className="notice warning shell-page-storage-warning" role="status">{pageStorageWarning}</div>}
        {switchFailure && <div className="notice warning shell-switch-failure" role="alert"><Icon name="alert" />
          <span>{`${TAB_NAMES[switchFailure.tab]} could not be shown: ${switchFailure.message}`}</span>
          <button type="button" className="button small" disabled={switching} onClick={() => void changeTab(switchFailure.tab)}>
            <Icon name="refresh" />Retry</button></div>}
      </header>

      <main className="workspace" ref={content}>
        {workspaceBackdrop.kind !== 'same' && <Backdrop className="is-workspace" backdrop={{ ...workspaceBackdrop, kind: workspaceBackdrop.kind }} image={appearance.workspaceImage} />}
        <section id="panel-settings" aria-label="Settings" tabIndex={-1} className="settings-panel" hidden={tab !== 'settings'}>
          {tab === 'settings' && <Settings section={settingsSection} target={settingsTarget} onTargetShown={clearSettingsTarget} folders={folders} folderError={folderError} onFoldersChange={value => { setFolders(value); setFolderError(null); }} native={native.state} metadata={metadata} branch={branch} library={replayLibrary} client={client} appearance={appearance}
            onResetPages={pagesReset()} />}
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
                  <button type="button" className="button primary" onClick={retryMap}><Icon name="refresh" />Retry map</button>
                  <button type="button" className="button" disabled={opening} onClick={() => void openRecording()}>
                    <Icon name="folder-open" />Open recording
                  </button>
                </div>
              </MapState>
            ) : !nativeReady ? (
              <MapState eyebrow="Native viewer" title="Preparing the map" busy>
                <p>Your map and playback controls will appear here.</p>
              </MapState>
            ) : path && nativePath && sameCampaignPath(path, nativePath) && native.state?.viewer.hour != null ? null : (
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
          <WidgetIconsContext.Provider value={widgetIconMenu.icons}><Dashboard page={page} resetToken={layoutReset} path={path} client={client} library={replayLibrary} onPrimaryPath={setDefaultPath} campaignHost={campaignHost} onOpenReady={registerOpen} active={tab === 'analytics'} mapActive={tab === 'map'} mapSwitching={switching} mapReady={!!nativeReady} cameraMode={native.state?.cameraMode ?? 'map'} cameraOverlays={native.state?.cameraOverlays ?? false} onMetadata={setMetadata} onBranch={setBranch} onMapError={reportMap} mapRetry={retry} onUnsaved={setLayoutsUnsaved} pages={pages} playbackHost={playbackHost} flyAircraft={native.state?.flyAircraft ?? null} sideHost={sideHost} /></WidgetIconsContext.Provider>
        </section>
      </main>
      <aside className="workspace-side" ref={setSideHost} aria-label="Map tools" hidden={tab !== 'map'} />
    </div>
  );
}
