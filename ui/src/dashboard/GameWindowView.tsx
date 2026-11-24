import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { MouseEvent, WheelEvent } from 'react';
import WidgetState from '../components/WidgetState';
import { hideTooltip, showTooltip } from '../charts/tooltip';
import type { DashboardClient, GameWindowRegion, GameWindowRender, GameWindowRequest } from './client';
import { useContentRevision } from './contentRevision';
import {
  gameWindowRequestKey,
  hitTestRegion,
  hitTestScrollRegion,
  nextScrollOffset,
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
    let media: MediaQueryList | null = null;
    const listen = () => {
      media = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      const onChange = () => {
        if (!active) return;
        const next = normalizeScale(window.devicePixelRatio);
        if (next !== current) {
          current = next;
          setDpr(next);
        }
        media?.removeEventListener('change', onChange);
        listen();
      };
      media.addEventListener('change', onChange);
    };
    listen();
    return () => { active = false; };
  }, []);
  return dpr;
}

export interface GameWindowViewProps {
  client: DashboardClient;
  request: Omit<GameWindowRequest, 'scale' | 'scroll'>;
  active: boolean;
  label: string;
  onDiagnostics?: (lines: string[]) => void;
  onShown?: (shown: ShownWindow | null) => void;
  onClose?: () => void;
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
  const [scroll, setScroll] = useState<Record<string, number>>({});
  const [render, setRender] = useState<GameWindowRender | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState(false);
  const [hoveredClose, setHoveredClose] = useState(false);
  const lastCompletedKey = useRef<string | null>(null);
  const imgRef = useRef<HTMLImageElement>(null);

  const fullRequest = useMemo<GameWindowRequest>(() => ({
    ...request,
    scale: dpr,
    scroll,
  }), [request, dpr, scroll]);

  const requestKey = useMemo(
    () => gameWindowRequestKey(fullRequest, contentRev),
    [fullRequest, contentRev],
  );

  useEffect(() => {
    if (!active) {
      hideTooltip(ownerId);
      return;
    }
    if (!client.renderGameWindow) {
      setError(new Error('Open BattlePlan in the desktop app to display game windows.'));
      setRender(null);
      return;
    }
    if (lastCompletedKey.current === requestKey && render !== null) {
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    client.renderGameWindow(fullRequest, controller.signal)
      .then(result => {
        if (controller.signal.aborted) return;
        lastCompletedKey.current = requestKey;
        setRender(result);
        setLoading(false);
        onDiagnostics?.([...result.warnings]);
        onShown?.({ hour: fullRequest.hour, readHour: result.readHour });
      })
      .catch(caught => {
        if (controller.signal.aborted) return;
        setLoading(false);
        setError(caught instanceof Error ? caught : new Error(String(caught)));
        onShown?.(null);
      });
    return () => { controller.abort(); };
  }, [active, client, fullRequest, requestKey, onDiagnostics, onShown]);

  useEffect(() => () => { hideTooltip(ownerId); }, [ownerId]);

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
    setHoveredClose(Boolean(hit?.close && onClose));
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
    if (hit?.close) {
      hideTooltip(ownerId);
      onClose?.();
    }
  }, [hitAt, onClose, ownerId]);

  const handleWheel = useCallback((event: WheelEvent<HTMLImageElement>) => {
    if (!render) return;
    const point = logicalPoint(event.clientX, event.clientY);
    if (!point) return;
    const hit = hitTestScrollRegion(render.regions, point[0], point[1]);
    if (!hit?.scroll) return;
    const next = nextScrollOffset(scroll, hit.scroll, event.deltaY, event.deltaX);
    if (!next) return;
    event.preventDefault();
    setScroll(next);
  }, [render, logicalPoint, scroll]);

  if (error) {
    return <WidgetState kind="error" failure={error} />;
  }
  if (!render) {
    return <WidgetState kind="loading" label={loading ? `Loading ${label}…` : `Preparing ${label}…`} />;
  }

  const [logicalWidth, logicalHeight] = render.logicalSize;
  return <div className="game-window-host">
    <div className="game-window-stage">
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
        onWheel={handleWheel}
      />
    </div>
    {render.artLabel && <div className="game-window-art-note">{render.artLabel}</div>}
  </div>;
}
