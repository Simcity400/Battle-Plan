import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { FocusEvent, KeyboardEvent, MouseEvent, PointerEvent, ReactNode } from 'react';
import WidgetState from '../components/WidgetState';
import { hideTooltip, showTooltip } from '../charts/tooltip';
import { useDetailWindowClose } from '../families/host/DetailWindowHost';
import type { DashboardClient, GameWindowRegion, GameWindowRender, GameWindowRequest, GameWindowScroll } from './client';
import { useContentRevision } from './contentRevision';
import {
  accessibleRegions,
  fitGameWindowSize,
  acceptRender,
  followGameWindowRender,
  frameShown,
  gameWindowRequestKey,
  gameWindowSubjectKey,
  hitTestClick,
  hitTestHover,
  hitTestScroll,
  isActivatableClose,
  listKeyInput,
  listPosition,
  normalizeGameWindowRequest,
  normalizeScale,
  planGameWindowRender,
  regionTooltipContent,
  scrollClickInput,
} from './gameWindowLogic';
import type { ListInput } from './gameWindowLogic';
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
  generation: number;
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
  generation: number;
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
  generation: number;
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
  generation,
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
  const latest = useRef({ key: requestKey, subject: subjectKey, generation, request: stableRequest, active, client });
  latest.current = { key: requestKey, subject: subjectKey, generation, request: stableRequest, active, client };

  const report = useCallback((done: CompletedRender) => {
    onDiagnosticsRef.current?.([...done.render.warnings]);
    onShownRef.current?.(shownOf(done));
  }, []);

  const launch = useCallback((request: GameWindowRequest, key: string, subject: string, generation: number) => {
    const renderWindow = latest.current.client.renderGameWindow;
    if (!renderWindow) return;
    const controller = new AbortController();
    const settled = { key, subject, generation };
    flight.current = { ...settled, controller };
    setLoading(true);
    const next = () => {
      const now = latest.current;
      if (followGameWindowRender(settled, now, completedRef.current)) launch(now.request, now.key, now.subject, now.generation);
      else setLoading(false);
    };
    renderWindow(request, controller.signal)
      .then(result => {
        if (controller.signal.aborted) return;
        flight.current = null;
        if (acceptRender(settled, latest.current)) {
          const done: CompletedRender = { ...settled, render: result, hour: request.hour };
          completedRef.current = done;
          setCompleted(done);
          setFailed(null);
          report(done);
        }
        next();
      })
      .catch(caught => {
        if (controller.signal.aborted) return;
        flight.current = null;
        if (acceptRender(settled, latest.current) && !followGameWindowRender(settled, latest.current, completedRef.current)) {
          setFailed({ key, error: caught instanceof Error ? caught : new Error(String(caught)) });
          onShownRef.current?.(null);
        }
        next();
      });
  }, [report]);

  useEffect(() => {
    const existing = completedRef.current;
    const want = { key: requestKey, subject: subjectKey, generation, active };
    const step = planGameWindowRender(want, flight.current, existing);
    if (existing && !frameShown(existing, want)) {
      hideTooltip(ownerId);
      setHoveredClose(false);
      onShownRef.current?.(null);
      onDiagnosticsRef.current?.([]);
    }
    if (step === 'idle') {
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
    if (step === 'reuse' && existing) {
      setFailed(null);
      report(existing);
      return;
    }
    if (step === 'wait') return;
    launch(stableRequest, requestKey, subjectKey, generation);
  }, [active, client, requestKey, subjectKey, generation, stableRequest, ownerId, launch, report]);

  useEffect(() => () => {
    flight.current?.controller.abort();
    flight.current = null;
    hideTooltip(ownerId);
  }, [ownerId]);

  const render = completed && frameShown(completed, { key: requestKey, subject: subjectKey, generation, active }) ? completed.render : null;
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
  const moveList = useCallback((scroll: GameWindowScroll, input: ListInput): boolean => {
    const now = wheelState.current;
    const current = now.positions?.[scroll.list];
    const next = listPosition(scroll, current, input);
    if (next === (current ?? scroll.position)) return false;
    hideTooltip(ownerId);
    setScrolled({ subject: now.subjectKey, positions: { ...now.positions, [scroll.list]: next } });
    return true;
  }, [ownerId]);
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
      moveList(list.scroll, { kind: 'wheel', delta: event.deltaY });
    };
    canvas.addEventListener('wheel', onWheel, { passive: false });
    return () => canvas.removeEventListener('wheel', onWheel);
  }, [shown, moveList]);

  const drag = useRef<{ pointer: number; scroll: GameWindowScroll; from: number } | null>(null);
  const handlePointerDown = useCallback((event: PointerEvent<HTMLElement>) => {
    hostRef.current?.focus({ preventScroll: true });
    const point = logicalPoint(event.clientX, event.clientY);
    const hit = point ? hitAt(event.clientX, event.clientY)?.click ?? null : null;
    const scroll = hit?.scroll;
    if (!point || !scroll || scroll.control.kind !== 'thumb' || hit.disabled) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { pointer: event.pointerId, scroll, from: point[scroll.vertical ? 1 : 0] };
  }, [hitAt, logicalPoint]);

  const handlePointerMove = useCallback((event: PointerEvent<HTMLElement>) => {
    const held = drag.current;
    if (!held || held.pointer !== event.pointerId) return;
    const point = logicalPoint(event.clientX, event.clientY);
    if (!point) return;
    moveList(held.scroll, { kind: 'drag', moved: point[held.scroll.vertical ? 1 : 0] - held.from });
  }, [logicalPoint, moveList]);

  const handlePointerUp = useCallback((event: PointerEvent<HTMLElement>) => {
    if (drag.current?.pointer !== event.pointerId) return;
    drag.current = null;
    event.currentTarget.releasePointerCapture(event.pointerId);
  }, []);

  const handleClick = useCallback((event: MouseEvent<HTMLElement>) => {
    const point = logicalPoint(event.clientX, event.clientY);
    const hit = hitAt(event.clientX, event.clientY)?.click ?? null;
    const input = render && point ? scrollClickInput(render.regions, hit, point[0], point[1]) : null;
    if (input && hit?.scroll) {
      moveList(hit.scroll, input);
    } else if (isActivatableClose(hit)) {
      hideTooltip(ownerId);
      effectiveClose?.();
    } else if (hit?.view && onView) {
      hideTooltip(ownerId);
      onView(hit.view);
    }
  }, [hitAt, logicalPoint, render, moveList, effectiveClose, onView, ownerId]);

  const handleListKey = useCallback((scroll: GameWindowScroll, event: KeyboardEvent<HTMLElement>) => {
    const input = listKeyInput(event.key);
    if (!input) return;
    event.preventDefault();
    event.stopPropagation();
    moveList(scroll, input);
  }, [moveList]);

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
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
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
          {focusRegions.map(({ region, rect, label: regionLabel, role }, index) => {
            const style = {
              left: `${(rect[0] / logicalWidth) * 100}%`,
              top: `${(rect[1] / logicalHeight) * 100}%`,
              width: `${(rect[2] / logicalWidth) * 100}%`,
              height: `${(rect[3] / logicalHeight) * 100}%`,
            };
            const scroll = region.scroll;
            if (role === 'list' && scroll) {
              return <div
                key={`${region.path}:${index}`}
                role="scrollbar"
                tabIndex={0}
                className="game-window-region-focus"
                style={style}
                aria-label={regionLabel}
                aria-orientation={scroll.vertical ? 'vertical' : 'horizontal'}
                aria-valuemin={0}
                aria-valuemax={scroll.range}
                aria-valuenow={positions?.[scroll.list] ?? scroll.position}
                onKeyDown={event => handleListKey(scroll, event)}
              />;
            }
            if (role === 'view' && onView && region.view) {
              const view = region.view;
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
                  onView(view);
                }}
              />;
            }
            if (role === 'close' && effectiveClose) {
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
            if (role !== 'note') return null;
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
    {render.text.length > 0 && <ul className="sr-only" aria-label={`${label} contents`}>
      {render.text.map((entry, index) => <li key={`${entry.path}:${index}`}>
        {entry.label}: {entry.value ?? `not recorded (${entry.missing ?? 'no reason given'})`}
      </li>)}
    </ul>}
    {render.artLabel && <div className="game-window-art-note">{render.artLabel}</div>}
  </>);
}
