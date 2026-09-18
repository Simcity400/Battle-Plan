import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import {
  chooseReplay,
  desktopAvailable,
  errorMessage,
  openReplay,
  seekMap,
} from './api';
import { Icon } from './Icons';
import { useDesktopState, useMapSurface } from './useNativeViewer';
import type { AnalyticsMemory } from './analytics/model';

const Analytics = lazy(() => import('./analytics/Analytics'));
type Tab = 'map' | 'analytics';
const tabs: Tab[] = ['map', 'analytics'];

export default function App() {
  const [tab, setTab] = useState<Tab>('map');
  const [retry, setRetry] = useState(0);
  const [path, setPath] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const pendingPath = useRef<string | null>(null);
  const lastNativePath = useRef<string | null>(null);
  const analyticsMemory = useRef<AnalyticsMemory | null>(null);
  const content = useRef<HTMLElement>(null);
  const native = useDesktopState(retry);
  // Native load/configuration failures keep their existing controls reachable.
  // Only shell-owned failures need the child hidden for the web recovery panel.
  const shellError = actionError ?? native.error;
  const boundsError = useMapSurface(content, tab === 'map' && !shellError, retry);
  const nativePath = native.state?.viewer.path ?? null;
  const mapError = actionError ?? boundsError ?? native.error ?? native.state?.error ?? null;

  useEffect(() => {
    const changed = lastNativePath.current !== nativePath;
    lastNativePath.current = nativePath;
    if (!nativePath) return;
    if (pendingPath.current && nativePath !== pendingPath.current) return;
    const acknowledged = pendingPath.current === nativePath;
    pendingPath.current = null;
    if (changed || acknowledged) setPath(nativePath);
  }, [nativePath, native.state?.viewer]);

  useEffect(() => {
    if (native.state?.error) pendingPath.current = null;
  }, [native.state?.error]);

  async function openRecording() {
    if (opening || !desktopAvailable) return;
    setOpening(true);
    setActionError(null);
    try {
      const chosen = await chooseReplay();
      if (!chosen) return;
      // Analytics can read the selected recording while native assets are loading.
      // Only viewer-state establishes which recording the map actually displays.
      pendingPath.current = chosen;
      setPath(chosen);
      await openReplay(chosen);
    } catch (cause) {
      pendingPath.current = null;
      setActionError(errorMessage(cause));
    } finally {
      setOpening(false);
    }
  }

  async function viewOnMap(branch: string, hour: number, revision: string) {
    if (!path) throw new Error('Choose a recording before opening a map hour.');
    pendingPath.current = path;
    try {
      await seekMap(path, revision, branch, hour);
    } catch (cause) {
      if (pendingPath.current === path) pendingPath.current = null;
      throw cause;
    }
    setActionError(null);
    setTab('map');
    requestAnimationFrame(() => document.getElementById('tab-map')?.focus());
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
    setTab(next);
    document.getElementById(`tab-${next}`)?.focus();
  }

  const nativeReady = native.state?.ready && native.state.running;
  const statusLabel = !desktopAvailable ? 'Browser' : mapError ? 'Issue' : nativeReady ? 'Ready' : 'Starting';

  return (
    <div className="app-shell">
      <aside className="side-rail" aria-label="Battle Plan navigation">
        <div className="brand-mark" title="Battle Plan" aria-label="Battle Plan">H4<span /></div>
        <div className="view-tabs" role="tablist" aria-label="Views" aria-orientation="vertical">
          {tabs.map((item) => (
            <button key={item} id={`tab-${item}`} type="button" role="tab"
              className="rail-button" aria-selected={tab === item} aria-controls={`panel-${item}`}
              tabIndex={tab === item ? 0 : -1} onClick={() => setTab(item)}
              onKeyDown={(event) => moveTab(event, item)}>
              <Icon name={item === 'map' ? 'map' : 'chart'} />
              <span>{item === 'map' ? 'Map' : 'Analytics'}</span>
            </button>
          ))}
        </div>
        <div className="rail-divider" />
        <button type="button" className="rail-button open-recording" disabled={!desktopAvailable || opening}
          onClick={() => void openRecording()} aria-label="Open recording"
          title={desktopAvailable ? 'Open a .battleplan recording' : 'Opening local recordings requires the desktop app'}>
          {opening ? <span className="spinner" /> : <Icon name="folder" />}
          <span>{opening ? 'Opening…' : <>Open<br />recording</>}</span>
        </button>
        <div className="rail-status" role="status"
          title={!desktopAvailable ? 'Browser preview. Native commands require the desktop app.' : mapError ?? 'Native viewer status'}>
          <span className={`status-dot ${mapError ? 'has-error' : nativeReady ? 'is-ready' : ''}`} />
          <span>{statusLabel}</span>
        </div>
      </aside>

      <main className="workspace" ref={content}>
        <section id="panel-map" role="tabpanel" aria-labelledby="tab-map" tabIndex={0}
          className="map-panel" hidden={tab !== 'map'}>
          <div className="map-placeholder">
            <div className="placeholder-icon"><Icon name="map" /></div>
            {!desktopAvailable ? (
              <>
                <span className="eyebrow">Browser preview</span>
                <h1>The map lives in the desktop app</h1>
                <p>Open Battle Plan on your desktop to load a recording and explore its native map.</p>
                <p className="subtle">The Map and Analytics tabs are available here to preview the interface.</p>
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
                <p>{path ? 'The native viewer is preparing the recorded map.' : 'Choose a .battleplan file to explore your campaign hour by hour.'}</p>
                {!path && <button type="button" className="button primary" disabled={opening} onClick={() => void openRecording()}>
                  <Icon name="folder" />Open recording
                </button>}
              </>
            )}
          </div>
        </section>

        <section id="panel-analytics" role="tabpanel" aria-labelledby="tab-analytics" tabIndex={0}
          className="analytics-panel" hidden={tab !== 'analytics'}>
          {tab === 'analytics' && (
            <Suspense fallback={<div className="panel-loading" role="status"><span className="spinner" />Loading analytics…</div>}>
              <Analytics key={path ?? 'empty'} path={path} memory={analyticsMemory}
                viewer={native.state?.viewer ?? null} viewerRunning={native.state?.running ?? false}
                viewerReady={native.state?.ready ?? false} onOpen={() => void openRecording()}
                opening={opening} openError={actionError} onViewOnMap={viewOnMap} />
            </Suspense>
          )}
        </section>
      </main>
    </div>
  );
}
