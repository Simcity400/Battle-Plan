import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { RefObject } from 'react';
import {
  desktopAvailable,
  errorMessage,
  getDesktopState,
  onViewerState,
  setMapView,
} from './api.ts';
import type { DesktopState, MapBounds, MapView } from './api';
import { engine } from '../engine/frame.ts';
import { coveredBy, mapHoles } from './mapOverlays';

export function useDesktopState(retry: number) {
  const [state, setState] = useState<DesktopState | null>(null);
  const [launchRecording, setLaunchRecording] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!desktopAvailable) return;
    let active = true;
    let eventVersion = 0;
    let unlisten: (() => void) | undefined;

    async function connect() {
      try {
        const stop = await onViewerState((next) => {
          if (!active) return;
          eventVersion += 1;
          setState(next);
          setError(null);
        });
        if (!active) {
          stop();
          return;
        }
        unlisten = stop;
        const beforeRead = eventVersion;
        const { launchRecording, ...snapshot } = await getDesktopState();
        if (!active) return;
        setLaunchRecording(launchRecording);
        if (beforeRead === eventVersion) setState(snapshot);
        setError(null);
      } catch (cause) {
        if (active) setError(errorMessage(cause));
      }
    }

    void connect();
    return () => {
      active = false;
      unlisten?.();
    };
  }, [retry]);

  return { state, error, launchRecording };
}

interface MapViewFailure {
  retry: number;
  message: string;
}

function createMapViewQueue(report: (failure: MapViewFailure) => void) {
  let pending: { view: MapView; retry: number; force: boolean } | null = null;
  let busy = false;
  let applied: string | null = null;
  let latestRetry = 0;
  let acceptedBounds: MapBounds = { x: 0, y: 0, width: 1, height: 1 };

  async function drain() {
    if (busy) return;
    busy = true;
    try {
      while (pending) {
        const next = pending;
        pending = null;
        const view = next.view.visible ? next.view : { visible: false, bounds: acceptedBounds };
        const key = JSON.stringify(view);
        if (!next.force && key === applied) continue;
        try {
          await setMapView(view);
          applied = key;
          acceptedBounds = view.bounds;
        } catch (cause) {
          applied = null;
          if (next.retry === latestRetry) report({ retry: next.retry, message: errorMessage(cause) });
        }
      }
    } finally {
      busy = false;
    }
  }

  return {
    send(view: MapView, retry: number, force = false) {
      latestRetry = retry;
      pending = { view, retry, force };
      void drain();
    },
  };
}

export function useMapSurface(
  element: RefObject<HTMLElement | null>,
  visible: boolean,
  retry: number,
  enabled = true,
): string | null {
  const [failure, setFailure] = useState<MapViewFailure | null>(null);
  const error = failure?.retry === retry ? failure.message : null;
  const surfaceVisible = visible && !error;
  const mounted = useRef(false);
  const queue = useMemo(
    () => createMapViewQueue((next) => { if (mounted.current) setFailure(next); }),
    [],
  );

  useLayoutEffect(() => {
    if (!desktopAvailable || !element.current || !enabled) return;
    mounted.current = true;
    const target = element.current;
    let frame: (() => void) | null = null;
    let resolution: MediaQueryList | null = null;
    let lastBounds: MapBounds = { x: 0, y: 0, width: 1, height: 1 };

    const measure = (force = false) => {
      const rect = target.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const x = Math.max(0, Math.round(rect.left * dpr));
      const y = Math.max(0, Math.round(rect.top * dpr));
      const width = Math.max(1, Math.round(rect.right * dpr) - x);
      const height = Math.max(1, Math.round(rect.bottom * dpr) - y);
      lastBounds = { x, y, width, height };
      queue.send({
        visible: surfaceVisible && !document.hidden && rect.width > 0 && rect.height > 0 && !mapHoles.covered() && !coveredBy(lastBounds, mapHoles.holes()),
        bounds: lastBounds,
      }, retry, force);
    };

    const schedule = () => {
      if (frame) return;
      frame = engine.next('measure', () => {
        frame = null;
        measure();
      });
    };
    const visibilityChanged = () => measure();

    const watchResolution = () => {
      resolution?.removeEventListener('change', resolutionChanged);
      resolution = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      resolution.addEventListener('change', resolutionChanged);
    };
    function resolutionChanged() {
      watchResolution();
      schedule();
    }

    const observer = new ResizeObserver(schedule);
    observer.observe(target);
    window.addEventListener('resize', schedule);
    window.visualViewport?.addEventListener('resize', schedule);
    document.addEventListener('visibilitychange', visibilityChanged);
    const stopHoles = mapHoles.subscribe(schedule);
    watchResolution();
    measure(true);

    return () => {
      mounted.current = false;
      observer.disconnect();
      frame?.();
      window.removeEventListener('resize', schedule);
      window.visualViewport?.removeEventListener('resize', schedule);
      document.removeEventListener('visibilitychange', visibilityChanged);
      stopHoles();
      resolution?.removeEventListener('change', resolutionChanged);
      queue.send({ visible: false, bounds: lastBounds }, retry);
    };
  }, [element, surfaceVisible, retry, queue, enabled]);

  return error;
}
