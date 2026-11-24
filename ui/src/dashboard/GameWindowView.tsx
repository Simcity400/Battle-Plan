import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { FocusEvent, KeyboardEvent, MouseEvent } from 'react';
import WidgetState from '../components/WidgetState';
import { hideTooltip, showTooltip } from '../charts/tooltip';
import type { DashboardClient, GameWindowRegion, GameWindowRender, GameWindowRequest } from './client';
import { useContentRevision } from './contentRevision';
import {
  accessibleRegions,
  gameWindowRequestKey,
  hitTestRegion,
  isActivatableClose,
  normalizeGameWindowRequest,
  normalizeScale,
  regionTooltipContent,
} from './gameWindowLogic';
import type { ShownWindow } from './gameWindows';
import './gameWindow.css';

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
}

interface CompletedRender {
  key: string;
  render: GameWindowRender;
  hour: number;
}

interface FailedRender {
  key: string;
  error: Error;
}

export default function GameWindowView({
  client,
  request,
  active,
  label,
  onDiagnostics,
  onShown,
  onClose,
}: GameWindowViewProps) {
  const ownerId = `game-window:${useId()}`;
  const dpr = useDevicePixelRatio();
  const contentRev = useContentRevision(client, active);
  const [completed, setCompleted] = useState<CompletedRender | null>(null);
  const [failed, setFailed] = useState<FailedRender | null>(null);
  const [loading, setLoading] = useState(false);
  const [hoveredClose, setHoveredClose] = useState(false);
  const imgRef = useRef<HTMLImageElement>(null);
  const completedRef = useRef<CompletedRender | null>(null);
  completedRef.current = completed;

  const normalized = normalizeGameWindowRequest({
    ...request,
    scale: dpr,
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

  useEffect(() => {
    if (!active) {
      hideTooltip(ownerId);
      return;
    }
    if (!client.renderGameWindow) {
      setFailed({
        key: requestKey,
        error: new Error('Open BattlePlan in the desktop app to display game windows.'),
      });
      setCompleted(null);
      return;
    }
    const existing = completedRef.current;
    if (existing && existing.key === requestKey) {
      setFailed(null);
      setLoading(false);
      onDiagnosticsRef.current?.([...existing.render.warnings]);
      onShownRef.current?.({ hour: existing.hour, readHour: existing.render.readHour });
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    setFailed(null);
    client.renderGameWindow(stableRequest, controller.signal)
      .then(result => {
        if (controller.signal.aborted) return;
        setCompleted({ key: requestKey, render: result, hour: stableRequest.hour });
        setFailed(null);
        setLoading(false);
        onDiagnosticsRef.current?.([...result.warnings]);
        onShownRef.current?.({ hour: stableRequest.hour, readHour: result.readHour });
      })
      .catch(caught => {
        if (controller.signal.aborted) return;
        setLoading(false);
        setFailed({
          key: requestKey,
          error: caught instanceof Error ? caught : new Error(String(caught)),
        });
        onShownRef.current?.(null);
      });
    return () => { controller.abort(); };
  }, [active, client, requestKey, stableRequest, ownerId]);

  useEffect(() => () => { hideTooltip(ownerId); }, [ownerId]);

  const render = completed?.render ?? null;
  const error = failed && failed.key === requestKey ? failed.error : null;

  const logicalPoint = useCallback((clientX: number, clientY: number): [number, number] | null => {
    const img = imgRef.current;
    if (!img || !render) return null;
    const rect = img.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    const x = ((clientX - rect.left) / rect.width) * render.logicalSize[0];
    const y = ((clientY - rect.top) / rect.height) * render.logicalSize[1];
    return [x, y];
  }, [render]);

  const hitAt = useCallback((clientX: number, clientY: number): GameWindowRegion | null => {
    if (!render) return null;
    const point = logicalPoint(clientX, clientY);
    if (!point) return null;
    return hitTestRegion(render.regions, point[0], point[1]);
  }, [render, logicalPoint]);

  const handleMove = useCallback((event: MouseEvent<HTMLImageElement>) => {
    const hit = hitAt(event.clientX, event.clientY);
    setHoveredClose(Boolean(isActivatableClose(hit) && onClose));
    const tooltip = hit ? regionTooltipContent(hit) : null;
    if (tooltip) {
      showTooltip(ownerId, tooltip, event.clientX, event.clientY, hit?.path);
    } else {
      hideTooltip(ownerId);
    }
  }, [hitAt, onClose, ownerId]);

  const handleLeave = useCallback(() => {
    setHoveredClose(false);
    hideTooltip(ownerId);
  }, [ownerId]);

  const handleClick = useCallback((event: MouseEvent<HTMLImageElement>) => {
    const hit = hitAt(event.clientX, event.clientY);
    if (isActivatableClose(hit)) {
      hideTooltip(ownerId);
      onClose?.();
    }
  }, [hitAt, onClose, ownerId]);

  const handleKeyDown = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape' && onClose) {
      event.preventDefault();
      event.stopPropagation();
      hideTooltip(ownerId);
      onClose();
    }
  }, [onClose, ownerId]);

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

  if (error) {
    return <WidgetState kind="error" failure={error} />;
  }
  if (!render) {
    return <WidgetState kind="loading" label={loading ? `Loading ${label}…` : `Preparing ${label}…`} />;
  }

  const [logicalWidth, logicalHeight] = render.logicalSize;
  return <div className="game-window-host" onKeyDown={handleKeyDown}>
    <div className="game-window-stage">
      <div className="game-window-canvas">
        <img
          ref={imgRef}
          className={`game-window-image${hoveredClose ? ' is-close-hover' : ''}`}
          src={`data:image/png;base64,${render.pngBase64}`}
          alt={label}
          width={logicalWidth}
          height={logicalHeight}
          draggable={false}
          onMouseMove={handleMove}
          onMouseLeave={handleLeave}
          onClick={handleClick}
        />
        <div className="game-window-regions" aria-label={`${label} controls`}>
          {focusRegions.map(({ region, rect, label: regionLabel, closeable }, index) => {
            const style = {
              left: `${(rect[0] / logicalWidth) * 100}%`,
              top: `${(rect[1] / logicalHeight) * 100}%`,
              width: `${(rect[2] / logicalWidth) * 100}%`,
              height: `${(rect[3] / logicalHeight) * 100}%`,
            };
            if (closeable && onClose) {
              return <button
                key={`${region.path}:${index}`}
                type="button"
                className="game-window-region-focus"
                style={style}
                aria-label={regionLabel}
                onFocus={event => handleRegionFocus(region, event)}
                onBlur={handleRegionBlur}
                onClick={() => {
                  hideTooltip(ownerId);
                  onClose();
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
  </div>;
}
