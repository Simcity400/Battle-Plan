import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import { flushSync } from 'react-dom';
import {
  answerCloseQuestion,
  closeRequests,
  desktopAvailable,
  errorMessage,
  leaveApp,
  openReplay,
  selectNativeView,
  setMapHoles,
} from './api.ts';
import { Icon } from '../components/Icons';
import EmptyState from '../components/EmptyState';
import type { IconName } from '../components/Icons';
import { loadable } from '../components/loadable';
import { defaultPages, loadPages, savePages } from '../dashboard/pages';
import type { PageEntry, PageId } from '../dashboard/pages';
import { clearLayouts, layoutsAtDefault } from '../dashboard/storage';
import { widgetDefinitions } from '../dashboard/definitions';
import { stableJson } from '../dashboard/model';
import { sameCampaignPath } from '../dashboard/campaigns';
import { actionFailure } from '../dashboard/queryError.ts';
import { DismissButton, FailureLine } from '../components/FailureLine';
import AnalyticsNav from './AnalyticsNav';
import BrandMark from '../components/BrandMark';
import { useDesktopState, useMapSurface } from './useNativeViewer';
import { nativeDashboardClient } from '../dashboard/nativeClient';
import { browserDashboardClient } from '../dashboard/browserClient';
import type { BranchChoice } from '../dashboard/Dashboard';
import { settingsSections } from './settingsMetadata';
import type { SettingsSection, SettingsTarget } from './settingsMetadata';
import type { ThemeEditing } from './Settings';
import { useAppearance } from './appearance/useAppearance';
import { useSidebarIconMenu } from './appearance/SidebarIconMenu';
import { StyledIcon } from './appearance/StyledIcon';
import { useWidgetIconMenu, WidgetIconsContext } from './appearance/WidgetIconMenu';
import Backdrop from './appearance/Backdrop';
import './shell.css';
import { useReplayLibrary } from '../dashboard/replayLibrary';
import { contentRevision } from '../dashboard/contentRevision';
import type { ContentFolders, ReplayMetadata } from './api';
import { mapHoles } from './mapOverlays';
import { useDiagnosticsShortcut } from './diagnosticsSetting';
import { RecordingNotice } from './recordingNotice';
import { commitZone } from './commitCount';
import { RestartGuardContext, UpdateButton } from '../update/UpdateControl';
import type { RestartGuard } from '../update/UpdateControl';
import { leavePrompt, leavingTitle, RESTARTING, unsavedDetail } from './unsaved';
import { draftSection, registerDraft, retryFailed, unsavedNow as pendingUnsaved, watchWrites, writesState } from './pendingWrites';
import { closeGuard, type CloseGuard, type CloseGuardFailure, type LeavingFor } from './closeGuard.ts';
import { keepLatest } from './latest.ts';
import type { LeavePrompt, Leaving } from './unsaved';
import { useUpdateState } from '../update/useUpdateState';

const browserRecordings = import.meta.env.DEV && !desktopAvailable;

const Dashboard = memo(loadable(() => import('../dashboard/Dashboard').then(module => module.default),
  () => <div className="panel-loading" role="status"><span className="spinner" />Loading overview…</div>));
const Settings = loadable(() => import('./Settings').then(module => module.default), () => <p role="status">Loading settings…</p>);
const SettingsNav = loadable(() => import('./Settings').then(module => module.SettingsNav));
type Tab = 'map' | 'analytics' | 'settings';
const TAB_NAMES: Record<Tab, string> = { map: 'the campaign map', analytics: 'Analytics', settings: 'Settings' };
const PAGE_KEY = 'battleplan.analytics-page.v1';
const SIDEBAR_KEY = 'battleplan.sidebar.v1';
const SIDEBAR_WIDTH_KEY = 'battleplan.sidebar-width.v1';
const SIDEBAR_WIDTH = { initial: 232, min: 200, max: 400, mainMin: 640 } as const;
const clampWidth = (width: number) => Math.round(Math.max(SIDEBAR_WIDTH.min, Math.min(SIDEBAR_WIDTH.max, window.innerWidth - SIDEBAR_WIDTH.mainMin, width)));

function useSidebarState() {
  const [hidden, setHidden] = useState(() => { try { return localStorage.getItem(SIDEBAR_KEY) === 'hidden'; } catch { return false; } });
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
  const saveWidth = (next: number) => { try { localStorage.setItem(SIDEBAR_WIDTH_KEY, String(next)); } catch {  } };
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
  const resetWidth = () => { setChosen(SIDEBAR_WIDTH.initial); try { localStorage.removeItem(SIDEBAR_WIDTH_KEY); } catch {  } };
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
      try { localStorage.setItem(SIDEBAR_KEY, value ? 'shown' : 'hidden'); } catch {  }
      return !value;
    });
  }, []);
  useEffect(() => {
    const key = (event: globalThis.KeyboardEvent) => {
      if (event.defaultPrevented || !(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey || event.key.toLowerCase() !== 'b') return;
      const target = event.target as HTMLElement | null;
      if (target?.closest('input:not([type="search"]), textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      event.preventDefault();
      toggle();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [toggle]);
  return { hidden, peek: hidden && peek, openPeek: () => setPeek(true), aside, toggle, width, resizing, startResize, resetWidth, resizeByKey };
}

function SidebarUpdate() {
  const { state } = useUpdateState();
  return <UpdateButton state={state} />;
}

function LeaveConfirm({ prompt, closing, confirming, failures, onCancel, onSave, onDiscard }: { prompt: LeavePrompt | null; closing: LeavingFor | null; confirming: boolean;
  failures: readonly CloseGuardFailure[]; onCancel: () => void; onSave: () => void; onDiscard: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const open = !!prompt || closing !== null;
  useLayoutEffect(() => {
    const element = dialog.current;
    if (!open || !element) return;
    if (!element.open) element.showModal();
    cancel.current?.focus({ preventScroll: true });
    return () => { if (element.open) element.close(); };
  }, [open]);
  const shownFailures = failures.map(failure => <FailureLine key={failure.key} className="is-top" message={failure.message} detail={failure.detail} onRetry={failure.retry} />);
  return <dialog ref={dialog} role="alertdialog" aria-labelledby="leave-confirm-title" aria-describedby={closing === null && prompt ? 'leave-confirm-detail' : undefined} className="leave-confirm"
    onKeyDown={event => { if (event.key === 'Escape') event.stopPropagation(); }}
    onCancel={event => { event.preventDefault(); if (closing === null) onCancel(); }} onClose={() => { if (prompt && closing === null) onCancel(); }}>
    {closing !== null ? <div className="menu-confirm" role="status">
      <strong id="leave-confirm-title">{leavingTitle(closing)}</strong>
      {shownFailures}
    </div> : prompt && <div className="menu-confirm">
      <strong id="leave-confirm-title">{prompt.title}</strong>
      <p id="leave-confirm-detail">{prompt.detail}</p>
      {shownFailures}
      <div className="menu-confirm-actions">
        <button ref={cancel} type="button" className="button small quiet" onClick={onCancel}>Cancel</button>
        <button type="button" className="button small is-danger" disabled={confirming} onClick={onDiscard}>{confirming ? RESTARTING : prompt.discard}</button>
        <button type="button" className="button small primary" disabled={confirming} onClick={onSave}>Save…</button>
      </div>
    </div>}
  </dialog>;
}

function themeUnsaved(editing: ThemeEditing | null): boolean {
  return !!editing && (!!editing.typing || JSON.stringify([editing.draft.name, editing.draft.variants]) !== JSON.stringify([editing.original.name, editing.original.variants]));
}

function MapState({ icon = 'map', title, children, busy = false, alert = false }: { icon?: IconName; title: string; children?: ReactNode; busy?: boolean; alert?: boolean }) {
  return <EmptyState icon={icon} title={title} busy={busy} tone={alert ? 'error' : undefined} role={alert ? 'alert' : 'status'}>{children}</EmptyState>;
}

export default function App() {
  const appearance = useAppearance();
  const iconMenu = useSidebarIconMenu(appearance);
  const widgetIconMenu = useWidgetIconMenu(appearance);
  const sidebar = useSidebarState();
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
  const [themeEditing, setThemeEditing] = useState<ThemeEditing | null>(null);
  const themeRef = useRef<ThemeEditing | null>(null);
  const editTheme = useMemo(() => keepLatest(themeRef, setThemeEditing), []);
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
  const [mapWarning, setMapWarning] = useState<string | null>(null);
  const choosePage = (next: PageId) => { setPage(next); try { localStorage.setItem(PAGE_KEY, next); } catch {  } };
  const changePages = (next: PageEntry[]) => { setPages(next); setPageStorageWarning(savePages(next)); };
  const deletePage = (id: PageId) => {
    const index = pages.findIndex(entry => entry.id === id);
    const next = pages.filter(entry => entry.id !== id);
    if (index < 0 || !next.length) return;
    changePages(next);
    clearLayouts(id);
    if (page === id) choosePage(next[Math.min(index, next.length - 1)].id);
  };
  const resetPages = () => {
    for (const entry of [...pages, ...defaultPages()]) clearLayouts(entry.id);
    const next = defaultPages();
    changePages(next);
    choosePage(next[0].id);
    setLayoutReset(count => count + 1);
  };
  const [widgetsLoading, setWidgetsLoading] = useState<'loading' | 'loaded' | 'failed'>('loading');
  const [unsavedPages, setUnsavedPages] = useState<readonly PageId[]>([]);
  const layoutsUnsaved = unsavedPages.length > 0;
  const [saveRequest, setSaveRequest] = useState(0);
  const [leaving, setLeaving] = useState<{ kind: Leaving; opener: Element | null; restart?: () => Promise<boolean>; confirmed?: boolean } | null>(null);
  const [closing, setClosing] = useState<LeavingFor | null>(null);
  const [guardFailures, setGuardFailures] = useState<readonly CloseGuardFailure[]>([]);
  const restartConfirmed = useRef(false);
  const guard = useRef<CloseGuard | null>(null);
  const layoutReader = useRef<(() => readonly PageId[]) | null>(null);
  const unsavedNow = useCallback(() => pendingUnsaved(), []);
  useEffect(() => {
    const layouts = registerDraft(() => (layoutReader.current?.().length ?? 0) > 0);
    const theme = registerDraft(() => themeUnsaved(themeRef.current));
    return () => { layouts(); theme(); };
  }, []);
  const writes = useSyncExternalStore(watchWrites, writesState);
  useEffect(() => {
    if (tab !== 'settings' || widgetDefinitions().length) return;
    import('../widgets/definitions').then(() => setWidgetsLoading('loaded'), () => setWidgetsLoading('failed'));
  }, [tab]);
  const pagesReset = (): (() => void) | null | undefined => {
    if (!widgetDefinitions().length) return widgetsLoading === 'failed' ? resetPages : null;
    return !layoutsUnsaved && stableJson(pages) === stableJson(defaultPages()) && pages.every(entry => layoutsAtDefault(entry.id)) ? undefined : resetPages;
  };
  const [playbackHost, setPlaybackHost] = useState<HTMLDivElement | null>(null);
  const [campaignHost, setCampaignHost] = useState<HTMLDivElement | null>(null);
  const [sideHost, setSideHost] = useState<HTMLElement | null>(null);
  const [metadata, setMetadata] = useState<ReplayMetadata | null>(null);
  const [branch, setBranch] = useState<BranchChoice | null>(null);
  const [folders, setFolders] = useState<ContentFolders | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  const [folderReads, setFolderReads] = useState(0);
  const setupChecked = useRef(false);
  const openCampaign = useRef<(() => void) | null>(null);
  const registerOpen = useCallback((open: (() => void) | null) => { openCampaign.current = open; }, []);
  const [retry, setRetry] = useState(0);
  const [path, setPath] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [switchFailure, setSwitchFailure] = useState<{ tab: Tab; message: string } | null>(null);
  const [mapFailures, setMapFailures] = useState<Record<string, string>>({});
  const reportMap = useCallback((operation: string, error: string | null) => setMapFailures(previous => {
    if ((previous[operation] ?? null) === error) return previous;
    const next = { ...previous };
    if (error == null) delete next[operation]; else next[operation] = error;
    return next;
  }), []);
  const actionError = Object.values(mapFailures)[0] ?? null;
  useEffect(() => desktopAvailable ? mapHoles.start(setMapHoles) : undefined, []);
  const pendingPath = useRef<string | null>(null);
  const defaultRequest = useRef(0);
  const lastNativePath = useRef<string | null>(null);
  const content = useRef<HTMLElement>(null);
  const header = useCallback((element: HTMLElement) => {
    const observer = new ResizeObserver(([entry]) => element.parentElement?.style.setProperty('--shell-header-height', `${entry.borderBoxSize[0].blockSize}px`));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const native = useDesktopState(retry);
  useEffect(() => { contentRevision(client).observe(folders); }, [client, folders?.game, folders?.user, folders?.contentReady]);
  useEffect(() => {
    if (!client.contentFolders) return;
    const controller = new AbortController();
    void client.contentFolders(controller.signal).then(value => { if (!controller.signal.aborted) { setFolders(value); setFolderError(null); } })
      .catch(cause => { if (!controller.signal.aborted) setFolderError(`Couldn’t read the folder settings: ${errorMessage(cause)}`); });
    return () => controller.abort();
  }, [client, folderReads]);
  useEffect(() => {
    if (!folders || setupChecked.current) return;
    setupChecked.current = true;
    if (folders.contentReady) return;
    let current = true;
    setSettingsSection('folders');
    void selectNativeView(true).then(() => {
      if (current) { setSwitchFailure(null); setTab('settings'); }
    }).catch(cause => { if (current) setSwitchFailure({ tab: 'settings', message: errorMessage(cause) }); });
    return () => { current = false; };
  }, [folders?.contentReady]);
  const shellError = actionError ?? native.error;
  const mapSurface = tab === 'map' && !!path && !shellError;
  const boundsError = useMapSurface(content, mapSurface && !leaving && !closing, retry, tab === 'map');
  const nativePath = native.state?.viewer.path ?? null;
  const mapError = actionError ?? boundsError ?? native.error ?? native.state?.error ?? null;

  useEffect(() => {
    const changed = lastNativePath.current !== nativePath;
    lastNativePath.current = nativePath;
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
      reportMap('open', null);
      if (next !== 'settings') lastView.current = next;
      setTab(next);
    } catch (cause: unknown) {
      if (selected) { pendingPath.current = null; await selectNativeView(tab !== 'map').catch(() => {}); }
      setSwitchFailure({ tab: next, message: errorMessage(cause) });
    } finally {
      setSwitching(false);
    }
  }

  function retryMap() {
    if (mapFailures.open != null) openDefault(path);
    setRetry(value => value + 1);
  }

  const themeChanged = themeUnsaved(themeEditing);
  const unsavedWork = layoutsUnsaved || themeChanged || writes !== null;
  const leavingNow = useRef(leaving);
  leavingNow.current = leaving;
  const editingToLeave = useRef<typeof leaving>(null);
  useEffect(() => {
    if (!unsavedWork) {
      const asked = leavingNow.current ?? editingToLeave.current;
      if (restartConfirmed.current || unsavedNow()) return;
      editingToLeave.current = null;
      setLeaving(null);
      if (asked?.kind === 'update' && asked.restart) void asked.restart().then(restarted => { if (!restarted) guard.current?.recheck(); });
      else if (asked) guard.current?.leave();
      return;
    }
    if (desktopAvailable) return;
    const unloading = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ''; };
    window.addEventListener('beforeunload', unloading);
    return () => window.removeEventListener('beforeunload', unloading);
  }, [unsavedWork, unsavedNow]);
  const unsavedNames = unsavedDetail(unsavedPages.map(id => pages.find(entry => entry.id === id)?.label ?? id), themeChanged ? themeEditing!.draft.name.trim() : null, writes);
  const prompt = leaving ? leavePrompt(leaving.kind, unsavedNames) : null;
  const leaveShown = !!prompt || closing !== null;
  useEffect(() => {
    const created = closeGuard({ requests: closeRequests, answer: answerCloseQuestion, leave: leaveApp }, {
      unsaved: unsavedNow,
      confirmed: () => restartConfirmed.current,
      prompt: () => setLeaving(previous => previous ?? { kind: 'close', opener: document.activeElement }),
      leaving: leavingFor => flushSync(() => setClosing(leavingFor)),
    }, setGuardFailures);
    guard.current = created;
    return () => { created.end(); if (guard.current === created) guard.current = null; };
  }, [unsavedNow]);
  const guardRestart = useCallback<RestartGuard>((restart, opener) => {
    if (!unsavedNow()) { void restart().then(restarted => { if (!restarted) guard.current?.recheck(); }); return; }
    setLeaving({ kind: 'update', opener, restart });
  }, [unsavedNow]);
  const stayHere = () => {
    if (closing) return;
    editingToLeave.current = null;
    restartConfirmed.current = false;
    const back = leaving?.opener;
    setLeaving(null);
    if (back instanceof HTMLElement && back.isConnected) requestAnimationFrame(() => back.focus({ preventScroll: true }));
  };
  const discardAndLeave = () => {
    if (leaving?.kind !== 'update' || !leaving.restart) { guard.current?.leave(); return; }
    const { restart } = leaving;
    restartConfirmed.current = true;
    setLeaving(previous => previous && { ...previous, confirmed: true });
    void restart().then(restarted => {
      if (!restarted) guard.current?.recheck();
      if (restarted || !restartConfirmed.current) return;
      restartConfirmed.current = false;
      setLeaving(null);
    });
  };
  const saveBeforeClose = () => {
    const section = draftSection() as SettingsSection | null;
    if (unsavedPages.length || themeChanged || section) { editingToLeave.current = leaving; setLeaving(null); }
    if (unsavedPages.length) {
      choosePage(unsavedPages[0]);
      setSaveRequest(count => count + 1);
      void changeTab('analytics');
      return;
    }
    if (themeChanged) {
      editTheme(previous => previous && { ...previous, focus: true });
      openSettings('appearance');
      return;
    }
    if (section) { openSettings(section); return; }
    retryFailed();
  };
  const everReady = useRef(false);

  const workspaceBackdrop = appearance.settings.workspaceBackdrop;
  const nativeReady = native.state?.ready && native.state.running;
  if (nativeReady) everReady.current = true;
  const mapOperation = Object.keys(mapFailures)[0];
  const titledFailure = (title: string, sentence: string, raw: string) => {
    const failure = actionFailure(sentence, raw);
    const repeated = failure.text.replace(/\.$/u, '').toLowerCase() === title.toLowerCase();
    return { title, failure: repeated ? { text: sentence, detail: raw } : failure };
  };
  const mapProblem = !mapError ? null
    : mapFailures.open != null ? titledFailure('The map couldn’t open this campaign', 'Couldn’t open the recording.', mapFailures.open)
    : titledFailure(everReady.current ? 'The map stopped' : 'The map couldn’t be prepared',
      mapOperation === 'seek' ? 'Couldn’t move the map to this hour.' : mapOperation ? 'Couldn’t apply the last map change.'
        : everReady.current ? 'The map closed unexpectedly.' : 'The map couldn’t start.', mapError);

  const openSettings = (section: SettingsSection, target: SettingsTarget | null = null) => {
    setSettingsSection(section);
    setSettingsTarget(target);
    if (tab !== 'settings') void changeTab('settings');
  };
  useEffect(() => {
    if (!editingToLeave.current || layoutsUnsaved || themeChanged) return;
    const section = draftSection() as SettingsSection | null;
    if (section) openSettings(section);
  }, [layoutsUnsaved, themeChanged]);
  const sectionLabel = settingsSections.find(item => item.id === settingsSection)?.label;

  return (
    <div className={`app-shell${sidebar.resizing ? ' is-resizing' : ''}`} data-sidebar={sidebar.peek ? 'peek' : sidebar.hidden ? 'hidden' : 'shown'} data-tab={tab}
      data-native-map={desktopAvailable && mapSurface || undefined}
      style={{ '--sidebar-width': sidebar.hidden ? '0px' : `${sidebar.width}px`, '--sidebar-peek-width': `${sidebar.width}px` } as CSSProperties}>
      <Backdrop backdrop={appearance.settings.backdrop} image={appearance.backdropImage} />
      <div ref={peekOverlay} popover="manual" className="sidebar-peek-overlay" aria-hidden="true" />
      {sidebar.hidden && <div className="sidebar-edge" aria-hidden="true" onPointerEnter={sidebar.openPeek} />}
      <aside ref={sidebar.aside} className="sidebar" aria-label="BattlePlan navigation" inert={sidebar.hidden && !sidebar.peek}
        onContextMenu={iconMenu.onContextMenu} onKeyDown={iconMenu.onKeyDown}>
        <div className="sidebar-header">
          <BrandMark size={30} /><span className="sidebar-wordmark">BattlePlan</span>
          <button type="button" className="icon-button sidebar-toggle" onClick={sidebar.toggle} aria-label={sidebar.hidden ? 'Keep sidebar open' : 'Hide sidebar'}
            data-tooltip={`${sidebar.hidden ? 'Keep sidebar open' : 'Hide sidebar'} · Ctrl+B`}><Icon name="sidebar" /></button>
        </div>
        <div className="sidebar-body">
        {tab === 'settings'
          ? <SettingsNav section={settingsSection} icons={appearance.settings.sidebarIcons} onBack={() => void changeTab(lastView.current)} onSection={section => openSettings(section)}
            onTarget={target => openSettings(target.section, target)} disabled={switching} />
          : <nav className="sidebar-menu sidebar-views" aria-label="Views">
            <button id="tab-map" type="button" className="sidebar-row" data-sidebar-icon="map" aria-current={tab === 'map' ? 'page' : undefined}
              disabled={switching} onClick={() => void changeTab('map')}><StyledIcon icon={iconMenu.icon('map')} fallback="map" colour={iconMenu.custom('map')} /><span>Campaign map</span></button>
            <AnalyticsNav pages={pages} page={page} active={tab === 'analytics'} disabled={switching} onPages={changePages} onDelete={deletePage} iconMenu={iconMenu}
              onChoose={id => { choosePage(id); void changeTab('analytics'); }} />
          </nav>}
        {tab !== 'settings' && folders && !folders.contentReady && <button type="button" className="sidebar-setup-note" disabled={switching}
          onClick={() => openSettings('folders')}><Icon name="folder" /><span>Choose your game folders to see flags and art</span><Icon name="chevron-right" /></button>}
        <div className="sidebar-campaign-host" ref={setCampaignHost} hidden={tab === 'settings'} />
        <div className="sidebar-footer">
          {tab !== 'settings' && <button id="tab-settings" type="button" className="icon-button sidebar-utility" data-sidebar-icon="settings" disabled={switching} aria-label="Settings" data-tooltip="Settings"
            onClick={() => void changeTab('settings')}><StyledIcon icon={iconMenu.icon('settings')} fallback="settings" colour={iconMenu.custom('settings')} /></button>}
          <RestartGuardContext.Provider value={guardRestart}><SidebarUpdate /></RestartGuardContext.Provider>
        </div>
        </div>
        {!sidebar.hidden && <div className="sidebar-rail" role="separator" aria-orientation="vertical" aria-label="Resize sidebar"
          tabIndex={0} aria-valuenow={sidebar.width} aria-valuemin={SIDEBAR_WIDTH.min} aria-valuemax={clampWidth(SIDEBAR_WIDTH.max)}
          onPointerDown={sidebar.startResize} onDoubleClick={sidebar.resetWidth} onKeyDown={sidebar.resizeByKey} />}
      </aside>
      {iconMenu.menu}
      {widgetIconMenu.menu}
      <header ref={header} className="shell-header">
        {sidebar.hidden && <button type="button" className="icon-button shell-sidebar-show" aria-label="Show sidebar" data-tooltip="Show sidebar · Ctrl+B"
          onClick={sidebar.toggle}><Icon name="sidebar" /></button>}
        <div className="shell-campaign-controls" ref={setPlaybackHost} hidden={tab === 'settings'} />
        {tab === 'settings' && <nav className="shell-location" aria-label="Location"><span>Settings</span><Icon name="chevron-right" /><strong aria-current="page">{sectionLabel}</strong></nav>}
        {tab !== 'settings' && <RecordingNotice partial={metadata?.partial} damaged={metadata?.damaged} path={metadata?.path} />}
        {pageStorageWarning && <div className="notice warning shell-page-storage-warning" role="status"><Icon name="alert" /><span>{pageStorageWarning}</span></div>}
        {mapWarning && <div className="notice warning shell-page-storage-warning" role="status"><Icon name="alert" /><span>{mapWarning}</span><DismissButton onDismiss={() => setMapWarning(null)} /></div>}
        {switchFailure && <FailureLine className="shell-switch-failure" message={`Couldn’t show ${TAB_NAMES[switchFailure.tab]}.`}
          detail={switchFailure.message} retryDisabled={switching} onRetry={() => void changeTab(switchFailure.tab)} />}
        {!leaveShown && guardFailures.map(failure => <FailureLine key={failure.key} className="shell-guard-failure" message={failure.message} detail={failure.detail} onRetry={failure.retry} />)}
      </header>

      <main className="workspace" ref={content}>
        {workspaceBackdrop.kind !== 'same' && <Backdrop className="is-workspace" backdrop={{ ...workspaceBackdrop, kind: workspaceBackdrop.kind }} image={appearance.workspaceImage} />}
        <section id="panel-settings" aria-label="Settings" tabIndex={-1} className="settings-panel" hidden={tab !== 'settings'}>
          {tab === 'settings' && <RestartGuardContext.Provider value={guardRestart}><Settings section={settingsSection} target={settingsTarget} onTargetShown={clearSettingsTarget} folders={folders} folderError={folderError} onFoldersRetry={() => setFolderReads(count => count + 1)} onFoldersChange={value => { setFolders(value); setFolderError(null); }} native={native.state} metadata={metadata} branch={branch} library={replayLibrary} client={client} appearance={appearance}
            onResetPages={pagesReset()} themeEditing={themeEditing} onThemeEditing={editTheme} /></RestartGuardContext.Provider>}
        </section>
        <section id="panel-map" aria-labelledby="tab-map" tabIndex={-1}
          className="map-panel" hidden={tab !== 'map'}>
          <div className="map-placeholder">
            {!desktopAvailable ? (
              <MapState title="The map is in the desktop app">
                <p>Open BattlePlan on your desktop to explore a campaign on its map.</p>
              </MapState>
            ) : mapProblem ? (
              <MapState icon="alert" title={mapProblem.title} alert>
                <p className="error-copy" data-detail={mapProblem.failure.detail}>{mapProblem.failure.text}</p>
                <div className="button-row centered">
                  <button type="button" className="button primary" onClick={retryMap}><Icon name="refresh" />Try again</button>
                  <button type="button" className="button" disabled={opening} onClick={() => void openRecording()}>
                    <Icon name="folder-open" />Open a recording…
                  </button>
                </div>
              </MapState>
            ) : !nativeReady ? (
              <MapState title="Preparing the map" busy>
                <p>The map and playback controls appear here when it’s ready.</p>
              </MapState>
            ) : path && nativePath && sameCampaignPath(path, nativePath) && native.state?.viewer.hour != null ? null : (
              <MapState title={path ? 'Opening the campaign' : 'No campaign open'} busy={!!path}>
                <p>{path ? 'The map appears here as soon as it’s ready.' : 'Choose a campaign from the library in the sidebar to explore it hour by hour.'}</p>
                {!path && <button type="button" className="button primary" disabled={opening} onClick={() => void openRecording()}>
                  <Icon name="folder-open" />Open a recording…
                </button>}
              </MapState>
            )}
          </div>
        </section>

        <section id="panel-analytics" aria-labelledby="tab-analytics" tabIndex={-1}
          className="analytics-panel" hidden={tab !== 'analytics'}>
          <WidgetIconsContext.Provider value={widgetIconMenu.icons}><Dashboard page={page} resetToken={layoutReset} path={path} client={client} library={replayLibrary} onPrimaryPath={setDefaultPath} campaignHost={campaignHost} onOpenReady={registerOpen} active={tab === 'analytics'} mapActive={tab === 'map'} mapSwitching={switching} mapReady={!!nativeReady} cameraMode={native.state?.cameraMode ?? 'map'} cameraOverlays={native.state?.cameraOverlays ?? false} onMetadata={setMetadata} onBranch={setBranch} onMapError={reportMap} onMapWarning={setMapWarning} mapRetry={retry} onUnsaved={setUnsavedPages} unsavedReader={layoutReader} saveRequest={saveRequest} pages={pages} playbackHost={playbackHost} flyAircraft={native.state?.flyAircraft ?? null} sideHost={sideHost} /></WidgetIconsContext.Provider>
        </section>
      </main>
      <aside className="workspace-side" ref={setSideHost} aria-label="Map tools" hidden={tab !== 'map'} />
      <LeaveConfirm prompt={prompt} closing={closing} confirming={!!leaving?.confirmed} failures={leaveShown ? guardFailures : []}
        onCancel={stayHere} onSave={saveBeforeClose} onDiscard={discardAndLeave} />
    </div>
  );
}

commitZone(App, 'shell');
