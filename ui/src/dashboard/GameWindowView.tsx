import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { FocusEvent, KeyboardEvent, MouseEvent, ReactNode } from 'react';
import WidgetState from '../components/WidgetState';
import { hideTooltip, showTooltip } from '../charts/tooltip';
import { useDetailWindowClose } from '../families/host/DetailWindowHost';
import type { DashboardClient, GameWindowRegion, GameWindowRender, GameWindowRequest } from './client';
import { useContentRevision } from './contentRevision';
import {
  accessibleRegions,
  fitGameWindowSize,
  followGameWindowRender,
  gameWindowRequestKey,
  gameWindowSubjectKey,
  hitTestClick,
  hitTestHover,
  hitTestScroll,
  isActivatableClose,
  normalizeGameWindowRequest,
  normalizeScale,
  planGameWindowRender,
  regionTooltipContent,
  wheelScrollPosition,
} from './gameWindowLogic';
import type { ShownWindow } from './gameWindows';
import './gameWindow.css';

function useViewportSize(): { width: number; height: number } {
  const [size, setSize] = useState(() => ({
    width: typeof window === 'undefined' ? 1440 : window.innerWidth,
    height: typeof window === 'undefined' ? 900 : window.innerHeight,
  }));
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const onResize = () => {
      setSize(prev => (
        prev.width === window.innerWidth && prev.height === window.innerHeight
          ? prev
          : { width: window.innerWidth, height: window.innerHeight }
      ));
    };
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  return size;
}

function useDevicePixelRatio(): number {
  const [dpr, setDpr] = useState(() => normalizeScale(typeof window === 'undefined' ? 1 : window.devicePixelRatio));
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    let active = true;
    let current = normalizeScale(window.devicePixelRatio);
    let cleanup: (() => void) | null = null;
    const listen = () => {
      const media = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      const onChange = () => {
        if (!active) return;
        const next = normalizeScale(window.devicePixelRatio);
        if (next !== current) {
          current = next;
          setDpr(next);
        }
        cleanup?.();
        cleanup = null;
        listen();
      };
      media.addEventListener('change', onChange);
      cleanup = () => { media.removeEventListener('change', onChange); };
    };
    listen();
    return () => {
      active = false;
      cleanup?.();
      cleanup = null;
    };
  }, []);
  return dpr;
}

export interface GameWindowViewProps {
  client: DashboardClient;
  request: Omit<GameWindowRequest, 'scale'>;
  active: boolean;
  label: string;
  onDiagnostics?: (lines: string[]) => void;
  onShown?: (shown: ShownWindow | null) => void;
  onClose?: () => void;
  onView?: (view: string) => void;
}

interface CompletedRender {
  key: string;
  subject: string;
  render: GameWindowRender;
  hour: number;
}

interface FailedRender {
  key: string;
  error: Error;
}

interface Flight {
  key: string;
  subject: string;
  controller: AbortController;
}

function shownOf(done: CompletedRender): ShownWindow {
  return {
    hour: done.hour,
    readHour: done.render.readHour,
    title: done.render.title ?? null,
    logicalSize: done.render.logicalSize,
  };
}

export default function GameWindowView({
  client,
  request,
  active,
  label,
  onDiagnostics,
  onShown,
  onClose,
  onView,
}: GameWindowViewProps) {
  const detailClose = useDetailWindowClose();
  const effectiveClose = detailClose ?? onClose;
  const ownerId = `game-window:${useId()}`;
  const dpr = useDevicePixelRatio();
  const viewport = useViewportSize();
  const contentRev = useContentRevision(client, active);
  const [completed, setCompleted] = useState<CompletedRender | null>(null);
  const [failed, setFailed] = useState<FailedRender | null>(null);
  const [loading, setLoading] = useState(false);
  const [hoveredClose, setHoveredClose] = useState(false);
  const [scrolled, setScrolled] = useState<{ subject: string; positions: Readonly<Record<string, number>> } | null>(null);
  const hostRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  const flight = useRef<Flight | null>(null);
  const completedRef = useRef<CompletedRender | null>(null);
  completedRef.current = completed;

  const subjectKey = gameWindowSubjectKey({ ...request, scale: dpr }, contentRev);
  const positions = scrolled?.subject === subjectKey ? scrolled.positions : undefined;
  const normalized = normalizeGameWindowRequest({
    ...request,
    scale: dpr,
    scroll: positions,
  });
  const requestKey = gameWindowRequestKey(normalized, contentRev);
  const stableRequestRef = useRef<{ key: string; value: GameWindowRequest } | null>(null);
  if (!stableRequestRef.current || stableRequestRef.current.key !== requestKey) {
    stableRequestRef.current = { key: requestKey, value: normalized };
  }
  const stableRequest = stableRequestRef.current.value;

  const onDiagnosticsRef = useRef(onDiagnostics);
  onDiagnosticsRef.current = onDiagnostics;
  const onShownRef = useRef(onShown);
  onShownRef.current = onShown;
  const latest = useRef({ key: requestKey, subject: subjectKey, request: stableRequest, active, client });
  latest.current = { key: requestKey, subject: subjectKey, request: stableRequest, active, client };

  const report = useCallback((done: CompletedRender) => {
    onDiagnosticsRef.current?.([...done.render.warnings]);
    onShownRef.current?.(shownOf(done));
  }, []);

  const launch = useCallback((request: GameWindowRequest, key: string, subject: string) => {
    const renderWindow = latest.current.client.renderGameWindow;
    if (!renderWindow) return;
    const controller = new AbortController();
    flight.current = { key, subject, controller };
    setLoading(true);
    setFailed(null);
    const next = () => {
      const now = latest.current;
      if (followGameWindowRender({ key, subject }, now, completedRef.current?.key ?? null)) {
        launch(now.request, now.key, now.subject);
      }
    };
    renderWindow(request, controller.signal)
      .then(result => {
        if (controller.signal.aborted) return;
        flight.current = null;
        const done: CompletedRender = { key, subject, render: result, hour: request.hour };
        completedRef.current = done;
        setCompleted(done);
        setLoading(false);
        if (latest.current.subject === subject) report(done);
        next();
      })
      .catch(caught => {
        if (controller.signal.aborted) return;
        flight.current = null;
        setLoading(false);
        if (followGameWindowRender({ key, subject }, latest.current, completedRef.current?.key ?? null)) {
          next();
          return;
        }
        setFailed({ key, error: caught instanceof Error ? caught : new Error(String(caught)) });
        onShownRef.current?.(null);
      });
  }, [report]);

  useEffect(() => {
    const existing = completedRef.current;
    const plan = planGameWindowRender(
      { key: requestKey, subject: subjectKey, active },
      flight.current,
      existing?.key ?? null,
    );
    if (plan.abort) {
      flight.current?.controller.abort();
      flight.current = null;
    }
    if (plan.step === 'idle') {
      setLoading(false);
      hideTooltip(ownerId);
      setHoveredClose(false);
      return;
    }
    if (!client.renderGameWindow) {
      setFailed({
        key: requestKey,
        error: new Error('Open BattlePlan in the desktop app to display game windows.'),
      });
      setCompleted(null);
      onShownRef.current?.(null);
      onDiagnosticsRef.current?.([]);
      return;
    }
    if (plan.step === 'reuse' && existing) {
      setFailed(null);
      setLoading(false);
      report(existing);
      return;
    }
    if (plan.step === 'wait') return;
    if (existing?.subject !== subjectKey) {
      hideTooltip(ownerId);
      setHoveredClose(false);
      onShownRef.current?.(null);
      onDiagnosticsRef.current?.([]);
    }
    launch(stableRequest, requestKey, subjectKey);
  }, [active, client, requestKey, subjectKey, stableRequest, ownerId, launch, report]);

  useEffect(() => () => {
    flight.current?.controller.abort();
    flight.current = null;
    hideTooltip(ownerId);
  }, [ownerId]);

  const render = completed && completed.subject === subjectKey ? completed.render : null;
  const error = failed && failed.key === requestKey ? failed.error : null;
  const previousLogicalSize = completed?.render.logicalSize ?? null;
  const previousHasArtLabel = Boolean(completed?.render.artLabel);

  const logicalPoint = useCallback((clientX: number, clientY: number): [number, number] | null => {
    const img = imgRef.current;
    if (!img || !render) return null;
    const rect = img.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    const x = ((clientX - rect.left) / rect.width) * render.logicalSize[0];
    const y = ((clientY - rect.top) / rect.height) * render.logicalSize[1];
    return [x, y];
  }, [render]);

  const hitAt = useCallback((clientX: number, clientY: number): { hover: GameWindowRegion | null; click: GameWindowRegion | null } | null => {
    if (!render) return null;
    const point = logicalPoint(clientX, clientY);
    if (!point) return null;
    return { hover: hitTestHover(render.regions, point[0], point[1]), click: hitTestClick(render.regions, point[0], point[1]) };
  }, [render, logicalPoint]);

  const handleMove = useCallback((event: MouseEvent<HTMLElement>) => {
    const hits = hitAt(event.clientX, event.clientY);
    const hit = hits?.hover ?? null;
    const click = hits?.click ?? null;
    setHoveredClose(Boolean((isActivatableClose(click) && effectiveClose) || (click?.view && onView)));
    const tooltip = hit ? regionTooltipContent(hit) : null;
    if (tooltip) {
      showTooltip(ownerId, tooltip, event.clientX, event.clientY, hit?.path);
    } else {
      hideTooltip(ownerId);
    }
  }, [hitAt, effectiveClose, onView, ownerId]);

  const handleLeave = useCallback(() => {
    setHoveredClose(false);
    hideTooltip(ownerId);
  }, [ownerId]);

  const wheelState = useRef({ render, logicalPoint, subjectKey, positions });
  wheelState.current = { render, logicalPoint, subjectKey, positions };
  const shown = render !== null;
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (event: WheelEvent) => {
      const now = wheelState.current;
      if (!now.render) return;
      const point = now.logicalPoint(event.clientX, event.clientY);
      const list = point ? hitTestScroll(now.render.regions, point[0], point[1]) : null;
      if (!list?.scroll) return;
      event.preventDefault();
      const next = wheelScrollPosition(list.scroll, now.positions?.[list.path], event.deltaY);
      if (next === (now.positions?.[list.path] ?? list.scroll.position)) return;
      hideTooltip(ownerId);
      setScrolled({ subject: now.subjectKey, positions: { ...now.positions, [list.path]: next } });
    };
    canvas.addEventListener('wheel', onWheel, { passive: false });
    return () => canvas.removeEventListener('wheel', onWheel);
  }, [shown, ownerId]);

  const handlePointerDown = useCallback(() => {
    hostRef.current?.focus({ preventScroll: true });
  }, []);

  const handleClick = useCallback((event: MouseEvent<HTMLElement>) => {
    const hit = hitAt(event.clientX, event.clientY)?.click ?? null;
    if (isActivatableClose(hit)) {
      hideTooltip(ownerId);
      effectiveClose?.();
    } else if (hit?.view && onView) {
      hideTooltip(ownerId);
      onView(hit.view);
    }
  }, [hitAt, effectiveClose, onView, ownerId]);

  const handleKeyDown = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape' && effectiveClose) {
      event.preventDefault();
      event.stopPropagation();
      hideTooltip(ownerId);
      effectiveClose();
    }
  }, [effectiveClose, ownerId]);

  const handleRegionFocus = useCallback((region: GameWindowRegion, event: FocusEvent<HTMLElement>) => {
    const tooltip = regionTooltipContent(region);
    if (!tooltip) {
      hideTooltip(ownerId);
      return;
    }
    const rect = event.currentTarget.getBoundingClientRect();
    showTooltip(ownerId, tooltip, rect.left + rect.width / 2, rect.bottom, region.path);
  }, [ownerId]);

  const handleRegionBlur = useCallback(() => {
    hideTooltip(ownerId);
  }, [ownerId]);

  const focusRegions = useMemo(
    () => (render ? accessibleRegions(render.regions, render.logicalSize) : []),
    [render],
  );

  const shell = (body: ReactNode) => <div
    ref={hostRef}
    className="game-window-host"
    tabIndex={-1}
    onKeyDown={handleKeyDown}
  >{body}</div>;

  if (error) {
    return shell(<WidgetState kind="error" failure={error} />);
  }
  if (!render) {
    const heldSize = previousLogicalSize
      ? fitGameWindowSize(previousLogicalSize, viewport, { width: 48, height: previousHasArtLabel ? 116 : 88 })
      : null;
    return shell(<div
      className="game-window-loading"
      style={heldSize ? { width: `${heldSize[0]}px`, height: `${heldSize[1]}px` } : undefined}
    >
      <WidgetState kind="loading" label={loading ? `Loading ${label}…` : `Preparing ${label}…`} />
    </div>);
  }

  const [logicalWidth, logicalHeight] = render.logicalSize;
  const [displayWidth, displayHeight] = fitGameWindowSize(
    render.logicalSize,
    viewport,
    { width: 48, height: render.artLabel ? 116 : 88 },
  );
  return shell(<>
    <div className="game-window-stage">
      <div
        ref={canvasRef}
        className={`game-window-canvas${hoveredClose ? ' is-close-hover' : ''}`}
        onPointerDown={handlePointerDown}
        onMouseEnter={handleMove}
        onMouseMove={handleMove}
        onMouseLeave={handleLeave}
        onClick={handleClick}
      >
        <img
          ref={imgRef}
          className="game-window-image"
          src={`data:image/png;base64,${render.pngBase64}`}
          alt={label}
          style={{
            width: `${displayWidth}px`,
            height: `${displayHeight}px`,
            aspectRatio: `${logicalWidth} / ${logicalHeight}`,
          }}
          draggable={false}
        />
        <div className="game-window-regions" aria-label={`${label} controls`}>
          {focusRegions.map(({ region, rect, label: regionLabel, closeable }, index) => {
            const style = {
              left: `${(rect[0] / logicalWidth) * 100}%`,
              top: `${(rect[1] / logicalHeight) * 100}%`,
              width: `${(rect[2] / logicalWidth) * 100}%`,
              height: `${(rect[3] / logicalHeight) * 100}%`,
            };
            if (closeable && effectiveClose) {
              return <button
                key={`${region.path}:${index}`}
                type="button"
                className="game-window-region-focus"
                style={style}
                aria-label={regionLabel}
                onFocus={event => handleRegionFocus(region, event)}
                onBlur={handleRegionBlur}
                onClick={event => {
                  event.stopPropagation();
                  hideTooltip(ownerId);
                  effectiveClose();
                }}
              />;
            }
            return <span
              key={`${region.path}:${index}`}
              role="note"
              tabIndex={0}
              className="game-window-region-focus"
              style={style}
              aria-label={regionLabel}
              onFocus={event => handleRegionFocus(region, event)}
              onBlur={handleRegionBlur}
            />;
          })}
        </div>
      </div>
    </div>
    {render.artLabel && <div className="game-window-art-note">{render.artLabel}</div>}
  </>);
}
