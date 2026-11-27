import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent } from 'react';
import { Icon } from '../components/Icons';
import { formatDateTime } from '../dashboard/calendar';
import { errorMessage } from '../shell/api';
import {
  chooseVideoExportPath,
  getMapExportSnapshot,
  getSurfaceExportTimelines,
  previewMapVideo,
  previewSurfaceVideo,
  startMapVideoExport,
  startSurfaceVideoExport,
  type CropRect,
  type EmbeddedMapSpec,
  type MapExportSnapshot,
  type MapVideoRequest,
  type Quality,
  type SurfaceTrackCoverage,
  type SurfaceVideoRequest,
} from './videoExportApi';
import {
  analyzeCadence,
  computeDurationSeconds,
  computeSpeedFromDuration,
  defaultAudioForSpeed,
  DIMENSION_PRESETS,
  estimateScheduleFrames,
  fitAspectCrop,
  formatExportDuration,
  FULL_CROP,
  isFullCrop,
  mergeRecordedTracks,
  mergeUnheldGaps,
  normalizeCrop,
  readabilityHint,
  snapEven,
  summarizeRange,
  validateDimensions,
} from './videoExportModel';
import type { ExportVideoTarget } from './ExportVideoContext';
import './exportVideo.css';

export type StartedExportDescriptor =
  | { kind: 'map'; request: MapVideoRequest; destination: string; label: string }
  | { kind: 'surface'; request: SurfaceVideoRequest; destination: string; label: string };

function bytesToPngDataUrl(bytes: Uint8Array): string {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return `data:image/png;base64,${btoa(binary)}`;
}

export default function ExportVideoDialog({
  target,
  busyExport,
  onClose,
  onStarted,
}: {
  target: ExportVideoTarget | null;
  busyExport: boolean;
  onClose: () => void;
  onStarted: (started: StartedExportDescriptor) => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [mapSnapshot, setMapSnapshot] = useState<MapExportSnapshot | null>(null);
  const [embeddedSnapshots, setEmbeddedSnapshots] = useState<Record<string, MapExportSnapshot>>({});
  const [surfaceTimelines, setSurfaceTimelines] = useState<Record<string, SurfaceTrackCoverage> | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const [fromHour, setFromHour] = useState(0);
  const [toHour, setToHour] = useState(240);
  const [fps, setFps] = useState<30 | 60>(60);
  const [speed, setSpeed] = useState(24);
  const [skipGaps, setSkipGaps] = useState(false);
  const [cutCardSeconds, setCutCardSeconds] = useState(1.0);
  const [presetId, setPresetId] = useState('1080p');
  const [width, setWidth] = useState(1920);
  const [height, setHeight] = useState(1080);
  const [quality, setQuality] = useState<Quality>('high');
  const [dateStamp, setDateStamp] = useState(true);
  const [audioEnabled, setAudioEnabled] = useState(true);
  const [audioManual, setAudioManual] = useState(false);
  const [audioMapWidgetId, setAudioMapWidgetId] = useState<string>('');
  const [crop, setCrop] = useState<CropRect>(FULL_CROP);
  const [cropMode, setCropMode] = useState(false);
  const [framing, setFraming] = useState<'full' | 'visible'>('full');
  const [reflowForPortrait, setReflowForPortrait] = useState(false);
  const [followPinned, setFollowPinned] = useState(false);
  const [zoomFull, setZoomFull] = useState(false);

  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [previewMinTextPx, setPreviewMinTextPx] = useState<number | null>(null);
  const [previewScale, setPreviewScale] = useState<number>(1);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const submission = useRef<AbortController | null>(null);
  const dismiss = () => {
    submission.current?.abort();
    onClose();
  };

  const dragRef = useRef<{
    kind: 'move' | 'resize';
    startX: number;
    startY: number;
    startCrop: CropRect;
    rect: DOMRect;
  } | null>(null);

  useLayoutEffect(() => {
    setSubmitting(false);
    setSubmitError(null);
    const dialog = dialogRef.current;
    if (dialog && target && !dialog.open) {
      dialog.showModal();
    } else if (dialog?.open && !target) {
      dialog.close();
    }
    return () => {
      submission.current?.abort();
      submission.current = null;
    };
  }, [target]);

  useEffect(() => {
    if (!target) {
      setMapSnapshot(null);
      setEmbeddedSnapshots({});
      setSurfaceTimelines(null);
      setPreviewUrl(null);
      setLoadError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    setSubmitError(null);
    setPreviewUrl(null);
    setPreviewError(null);
    setPreviewMinTextPx(null);
    setPreviewScale(1);
    setAudioManual(false);
    setCrop(FULL_CROP);
    setCropMode(false);
    setZoomFull(false);
    setFraming('full');
    setReflowForPortrait(false);
    setFollowPinned(false);

    if (target.kind === 'map') {
      void getMapExportSnapshot(target.viewId)
        .then(snap => {
          if (cancelled) return;
          setMapSnapshot(snap);
          setFromHour(snap.firstHour);
          setToHour(snap.lastHour);
          const span = Math.max(1, snap.lastHour - snap.firstHour);
          const initialSpeed = span > 2400 ? 72 : 24;
          setSpeed(initialSpeed);
          setAudioEnabled(defaultAudioForSpeed(initialSpeed) && !snap.audioMuted);
          setPresetId('1080p');
          setWidth(1920);
          setHeight(1080);
        })
        .catch(cause => {
          if (!cancelled) setLoadError(errorMessage(cause));
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    } else {
      setMapSnapshot(null);
      setSurfaceTimelines(null);
      const ctx = target.context;
      setFromHour(ctx.firstHour);
      setToHour(ctx.lastHour);
      const span = Math.max(1, ctx.lastHour - ctx.firstHour);
      const initialSpeed = span > 2400 ? 72 : 24;
      setSpeed(initialSpeed);
      setPresetId('1080p');
      setWidth(1920);
      setHeight(1080);
      const advancing = ctx.mapWidgets.filter(mw => mw.timeMode === 'follow');
      const firstMap = advancing[0]?.widgetId ?? '';
      setAudioMapWidgetId(defaultAudioForSpeed(initialSpeed) ? firstMap : '');
      const timelinesPromise = ctx.tracks.length
        ? getSurfaceExportTimelines(
            ctx.tracks.map(t => ({
              path: t.path,
              revision: t.revision || null,
              branch: Number.isFinite(Number(t.branchId)) ? Number(t.branchId) : 0,
              axisOffset: t.axisOffset,
            })),
          )
        : Promise.resolve([]);
      const mapSnapshotsPromise = Promise.all(
        ctx.mapWidgets.map(async mw => {
          try {
            const snap = await getMapExportSnapshot(mw.widgetId);
            return [mw.widgetId, snap] as const;
          } catch {
            return null;
          }
        }),
      );
      void Promise.all([timelinesPromise, mapSnapshotsPromise])
        .then(([timelines, entries]) => {
          if (cancelled) return;
          const byId: Record<string, SurfaceTrackCoverage> = {};
          ctx.tracks.forEach((track, idx) => {
            const cov = timelines[idx];
            if (cov) byId[track.id] = cov;
          });
          setSurfaceTimelines(byId);
          const map: Record<string, MapExportSnapshot> = {};
          for (const entry of entries) {
            if (entry) map[entry[0]] = entry[1];
          }
          setEmbeddedSnapshots(map);
        })
        .catch(cause => {
          if (!cancelled) {
            setSurfaceTimelines(null);
            setLoadError(errorMessage(cause));
          }
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    }

    return () => {
      cancelled = true;
    };
  }, [target]);

  const advancingMapWidgets = useMemo(() => {
    if (!target || target.kind === 'map') return [];
    return target.context.mapWidgets.filter(
      mw => mw.timeMode === 'follow' || (mw.timeMode === 'pin' && followPinned),
    );
  }, [target, followPinned]);

  useEffect(() => {
    if (!target || target.kind === 'map') return;
    if (audioMapWidgetId && !advancingMapWidgets.some(mw => mw.widgetId === audioMapWidgetId)) {
      setAudioMapWidgetId('');
    }
  }, [target, advancingMapWidgets, audioMapWidgetId]);

  useEffect(() => {
    if (audioManual || !target) return;
    if (target.kind === 'map' && mapSnapshot) {
      setAudioEnabled(defaultAudioForSpeed(speed) && !mapSnapshot.audioMuted);
    } else if (target.kind !== 'map') {
      const firstMap = advancingMapWidgets[0]?.widgetId ?? '';
      setAudioMapWidgetId(defaultAudioForSpeed(speed) ? firstMap : '');
    }
  }, [speed, audioManual, target, mapSnapshot, advancingMapWidgets]);

  const activeSurfaceTracks = useMemo(() => {
    if (!target || target.kind === 'map') return [];
    const ctx = target.context;
    if (target.kind === 'widget') {
      return ctx.tracks;
    }
    const chosenIds = new Set<string>(ctx.followTrackIds);
    if (followPinned) {
      for (const id of ctx.pinnedTrackIds) chosenIds.add(id);
    }
    let tracks = ctx.tracks.filter(t => chosenIds.has(t.id));
    if (tracks.length === 0) {
      const fallbackIds = new Set<string>(ctx.fallbackTrackIds);
      tracks = ctx.tracks.filter(t => fallbackIds.has(t.id));
    }
    return tracks.length > 0 ? tracks : ctx.tracks;
  }, [target, followPinned]);

  const activeSurfaceTimelines = useMemo(() => {
    if (!surfaceTimelines) return null;
    const out: SurfaceTrackCoverage[] = [];
    for (const track of activeSurfaceTracks) {
      const cov = surfaceTimelines[track.id];
      if (cov) out.push(cov);
    }
    return out;
  }, [surfaceTimelines, activeSurfaceTracks]);

  const bounds = useMemo(() => {
    if (!target) return { firstHour: 0, lastHour: 0, currentHour: 0, referenceOffset: 0, maxTexture: 8192, viewport: [1920, 1080] as [number, number] };
    if (target.kind === 'map') {
      return {
        firstHour: mapSnapshot?.firstHour ?? 0,
        lastHour: mapSnapshot?.lastHour ?? 0,
        currentHour: mapSnapshot?.currentHour ?? 0,
        referenceOffset: 0,
        maxTexture: mapSnapshot?.maxTextureDimension ?? 8192,
        viewport: mapSnapshot?.viewport ?? ([1920, 1080] as [number, number]),
      };
    }
    const firstHour = activeSurfaceTracks.length
      ? Math.min(...activeSurfaceTracks.map(t => t.firstHour))
      : target.context.firstHour;
    const lastHour = activeSurfaceTracks.length
      ? Math.max(...activeSurfaceTracks.map(t => t.lastHour))
      : target.context.lastHour;
    return {
      firstHour,
      lastHour,
      currentHour: target.context.currentHour,
      referenceOffset: target.context.referenceOffset,
      maxTexture: 8192,
      viewport: [target.context.sourceWidth, target.context.sourceHeight] as [number, number],
    };
  }, [target, mapSnapshot, activeSurfaceTracks]);

  useEffect(() => {
    if (!target || target.kind !== 'page') return;
    setFromHour(prev => Math.min(Math.max(prev, bounds.firstHour), bounds.lastHour));
    setToHour(prev => Math.min(Math.max(prev, bounds.firstHour), bounds.lastHour));
  }, [target, bounds.firstHour, bounds.lastHour]);

  const ready =
    target !== null &&
    !loadError &&
    (target.kind === 'map'
      ? mapSnapshot !== null
      : activeSurfaceTimelines !== null && activeSurfaceTimelines.length > 0);

  const previewGenRef = useRef(0);

  const mergedSurface = useMemo(() => {
    if (!target || target.kind === 'map' || !activeSurfaceTimelines) return null;
    const stretches = mergeRecordedTracks(activeSurfaceTimelines.map(t => t.stretches));
    const unheld = mergeUnheldGaps(
      stretches,
      activeSurfaceTimelines.map(t => t.unheld),
      fromHour,
      toHour,
    );
    return { stretches, unheld };
  }, [target, activeSurfaceTimelines, fromHour, toHour]);

  const activeStretches = useMemo(() => {
    if (!target) return null;
    if (target.kind === 'map') return mapSnapshot?.stretches ?? null;
    return mergedSurface?.stretches ?? null;
  }, [target, mapSnapshot, mergedSurface]);

  const coverage = useMemo(() => {
    if (!target) return { spanHours: 0, recordedHours: 0, missingHours: 0, gapCount: 0 };
    if (target.kind === 'map') {
      if (!mapSnapshot) return { spanHours: 0, recordedHours: 0, missingHours: 0, gapCount: 0 };
      return summarizeRange(mapSnapshot.stretches, mapSnapshot.unheld, fromHour, toHour);
    }
    if (!mergedSurface) return { spanHours: 0, recordedHours: 0, missingHours: 0, gapCount: 0 };
    return summarizeRange(mergedSurface.stretches, mergedSurface.unheld, fromHour, toHour);
  }, [target, mapSnapshot, mergedSurface, fromHour, toHour]);

  const estimatedFrames = useMemo(() => {
    if (!activeStretches) return 0;
    return estimateScheduleFrames(activeStretches, fromHour, toHour, fps, speed, skipGaps, cutCardSeconds);
  }, [activeStretches, fromHour, toHour, fps, speed, skipGaps, cutCardSeconds]);

  const durationSeconds = useMemo(
    () =>
      computeDurationSeconds(
        coverage,
        speed,
        skipGaps,
        cutCardSeconds,
        fps,
        activeStretches ?? undefined,
        fromHour,
        toHour,
      ),
    [coverage, speed, skipGaps, cutCardSeconds, fps, activeStretches, fromHour, toHour],
  );

  const cadence = useMemo(() => analyzeCadence(speed, fps), [speed, fps]);

  const dimensionError = useMemo(
    () => validateDimensions(width, height, bounds.maxTexture),
    [width, height, bounds.maxTexture],
  );

  const buildMapRequest = useCallback(
    (useFullFrameForCropEdit: boolean): MapVideoRequest | null => {
      if (!mapSnapshot) return null;
      const evenW = snapEven(width);
      const evenH = snapEven(height);
      let previewW = evenW;
      let previewH = evenH;
      if (useFullFrameForCropEdit) {
        const srcW = Math.max(16, mapSnapshot.viewport[0] || 1280);
        const srcH = Math.max(16, mapSnapshot.viewport[1] || 720);
        const factor = Math.min(1, 1280 / srcW, 720 / srcH);
        previewW = snapEven(srcW * factor);
        previewH = snapEven(srcH * factor);
      }
      return {
        path: mapSnapshot.path,
        revision: mapSnapshot.revision,
        branch: mapSnapshot.branch,
        fromHour: Math.min(fromHour, toHour),
        toHour: Math.max(fromHour, toHour),
        fps,
        hoursPerSecond: speed,
        skipGaps,
        cutCardSeconds,
        width: previewW,
        height: previewH,
        quality,
        backend: 'auto',
        ffmpegPath: null,
        placement: mapSnapshot.placement,
        sourceViewport: mapSnapshot.viewport,
        crop: useFullFrameForCropEdit || isFullCrop(crop) ? null : normalizeCrop(crop),
        uiScale: mapSnapshot.uiScale,
        mapMode: mapSnapshot.mapMode,
        perspective: mapSnapshot.perspective,
        perspectiveSlot: mapSnapshot.perspectiveSlot ?? null,
        fog: mapSnapshot.fog,
        dayNight: mapSnapshot.dayNight,
        alliedPlans: mapSnapshot.alliedPlans,
        counterColorsCountry: mapSnapshot.counterColorsCountry,
        natoCounters: mapSnapshot.natoCounters,
        mapSettings: mapSnapshot.mapSettings,
        dateStamp,
        audioEnabled,
        allowUnverified: false,
      };
    },
    [
      mapSnapshot,
      fromHour,
      toHour,
      fps,
      speed,
      skipGaps,
      cutCardSeconds,
      width,
      height,
      quality,
      crop,
      dateStamp,
      audioEnabled,
    ],
  );

  const buildSurfaceRequest = useCallback((): SurfaceVideoRequest | null => {
    if (!target || target.kind === 'map' || !activeSurfaceTimelines || activeSurfaceTimelines.length === 0) return null;
    const ctx = target.context;
    const evenW = snapEven(width);
    const evenH = snapEven(height);
    const embeddedMaps: EmbeddedMapSpec[] = ctx.mapWidgets.map(mw => {
      const snap = embeddedSnapshots[mw.widgetId];
      return {
        widgetId: mw.widgetId,
        title: mw.title,
        path: snap?.path ?? mw.path,
        revision: snap?.revision ?? mw.revision,
        branch: snap?.branch ?? mw.branch,
        axisOffset: mw.axisOffset,
        timeMode: mw.timeMode,
        fixedHour: mw.fixedHour,
        placement: snap?.placement ?? null,
        sourceViewport: snap?.viewport ?? null,
        uiScale: snap?.uiScale ?? 1.0,
        mapMode: snap?.mapMode ?? null,
        perspective: snap?.perspective ?? mw.perspective,
        perspectiveSlot: snap?.perspectiveSlot ?? mw.perspectiveSlot ?? null,
        fog: snap?.fog ?? mw.fog,
        dayNight: snap?.dayNight ?? null,
        alliedPlans: snap?.alliedPlans ?? null,
        counterColorsCountry: snap?.counterColorsCountry ?? null,
        natoCounters: snap?.natoCounters ?? null,
        mapSettings: snap?.mapSettings,
      };
    });
    const effectiveAudioMapId = advancingMapWidgets.some(mw => mw.widgetId === audioMapWidgetId)
      ? audioMapWidgetId
      : null;
    return {
      kind: target.kind,
      fromHour: Math.min(fromHour, toHour),
      toHour: Math.max(fromHour, toHour),
      referenceOffset: ctx.referenceOffset,
      fps,
      hoursPerSecond: speed,
      skipGaps,
      cutCardSeconds,
      width: evenW,
      height: evenH,
      quality,
      backend: 'auto',
      ffmpegPath: null,
      framing: target.kind === 'page' ? framing : 'visible',
      reflowForPortrait: target.kind === 'page' && framing === 'visible' && reflowForPortrait,
      sourceWidth: Math.max(320, ctx.sourceWidth),
      sourceHeight: Math.max(240, ctx.sourceHeight),
      scrollTop: Math.max(0, ctx.scrollTop),
      dateStamp,
      followPinned,
      hasRankMarkerTables: ctx.hasRankMarkerTables,
      audioMapWidgetId: effectiveAudioMapId,
      allowUnverified: false,
      tracks: activeSurfaceTimelines,
      embeddedMaps,
      payload: {
        template: target.template,
        ...(target.kind === 'widget' ? { widget: target.widget } : {}),
        campaigns: ctx.campaigns,
        selectedCountries: ctx.selectedCountries,
        selectedBranches: ctx.selectedBranches,
        observed: ctx.observed,
        defaultRoleId: ctx.defaultRoleId,
      },
    };
  }, [
    target,
    embeddedSnapshots,
    activeSurfaceTimelines,
    advancingMapWidgets,
    fromHour,
    toHour,
    fps,
    speed,
    skipGaps,
    cutCardSeconds,
    width,
    height,
    quality,
    framing,
    reflowForPortrait,
    dateStamp,
    followPinned,
    audioMapWidgetId,
  ]);

  const renderPreview = useCallback(async () => {
    if (!target || !ready || dimensionError) return;
    const gen = ++previewGenRef.current;
    setPreviewBusy(true);
    setPreviewError(null);
    try {
      const previewHour = Math.min(Math.max(bounds.currentHour, Math.min(fromHour, toHour)), Math.max(fromHour, toHour));
      if (target.kind === 'map') {
        const req = buildMapRequest(cropMode);
        if (!req) return;
        const pngBytes = await previewMapVideo(req, previewHour);
        if (gen !== previewGenRef.current) return;
        const nextUrl = bytesToPngDataUrl(pngBytes);
        setPreviewUrl(nextUrl);
      } else {
        const req = buildSurfaceRequest();
        if (!req) return;
        const result = await previewSurfaceVideo(req, previewHour);
        if (gen !== previewGenRef.current) return;
        const nextUrl = `data:image/png;base64,${result.pngBase64}`;
        setPreviewMinTextPx(result.minTextPx);
        setPreviewScale(result.scale);
        setPreviewUrl(nextUrl);
      }
    } catch (cause) {
      if (gen !== previewGenRef.current) return;
      setPreviewError(errorMessage(cause));
    } finally {
      if (gen === previewGenRef.current) {
        setPreviewBusy(false);
      }
    }
  }, [target, ready, dimensionError, bounds.currentHour, fromHour, toHour, buildMapRequest, buildSurfaceRequest, cropMode]);

  useEffect(() => {
    previewGenRef.current += 1;
    setPreviewUrl(null);
    setPreviewError(null);
    setPreviewBusy(false);
  }, [target]);

  useEffect(() => {
    previewGenRef.current += 1;
    if (!target || !ready || loading || dimensionError) {
      if (dimensionError) {
        setPreviewUrl(null);
        setPreviewBusy(false);
      }
      return;
    }
    const timer = window.setTimeout(() => {
      void renderPreview();
    }, 260);
    return () => window.clearTimeout(timer);
  }, [target, ready, loading, dimensionError, renderPreview]);

  useEffect(() => {
    return () => {
      setPreviewUrl(null);
    };
  }, []);

  const choosePreset = (id: string) => {
    setPresetId(id);
    if (id === 'match') {
      setWidth(snapEven(bounds.viewport[0] || 1920));
      setHeight(snapEven(bounds.viewport[1] || 1080));
      return;
    }
    const found = DIMENSION_PRESETS.find(item => item.id === id);
    if (found) {
      setWidth(found.width);
      setHeight(found.height);
    }
  };

  const applyAspectSafeFrame = () => {
    if (!mapSnapshot) return;
    setCrop(fitAspectCrop(mapSnapshot.viewport, width, height));
    setCropMode(true);
  };

  const startCropDrag = (kind: 'move' | 'resize', event: PointerEvent<HTMLDivElement>) => {
    const parent = event.currentTarget.closest('.export-crop-overlay');
    if (!parent) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = {
      kind,
      startX: event.clientX,
      startY: event.clientY,
      startCrop: crop,
      rect: parent.getBoundingClientRect(),
    };
  };

  const moveCropDrag = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.rect.width <= 0 || drag.rect.height <= 0) return;
    const dx = (event.clientX - drag.startX) / drag.rect.width;
    const dy = (event.clientY - drag.startY) / drag.rect.height;
    if (drag.kind === 'move') {
      setCrop(
        normalizeCrop({
          ...drag.startCrop,
          x: drag.startCrop.x + dx,
          y: drag.startCrop.y + dy,
        }),
      );
    } else {
      const targetAspect = width / Math.max(1, height);
      const stageAspect = drag.rect.width / drag.rect.height;
      const nextHeight = Math.min(1 - drag.startCrop.y, Math.max(0.1, drag.startCrop.height + dy));
      const nextWidth = Math.min(1 - drag.startCrop.x, Math.max(0.1, (nextHeight * targetAspect) / stageAspect));
      setCrop(
        normalizeCrop({
          x: drag.startCrop.x,
          y: drag.startCrop.y,
          width: nextWidth,
          height: nextHeight,
        }),
      );
    }
  };

  const endCropDrag = () => {
    dragRef.current = null;
  };

  const handleExport = async () => {
    if (!target || !ready || dimensionError || busyExport || submission.current) return;
    const current = new AbortController();
    submission.current = current;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const safeName = target.label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'battleplan-video';
      const destination = await chooseVideoExportPath(`${safeName}.mp4`);
      if (!destination || current.signal.aborted) return;
      if (target.kind === 'map') {
        const req = buildMapRequest(false);
        if (!req) return;
        await startMapVideoExport(req, destination, target.label);
        onStarted({ kind: 'map', request: req, destination, label: target.label });
      } else {
        const req = buildSurfaceRequest();
        if (!req) return;
        await startSurfaceVideoExport(req, destination, target.label);
        onStarted({ kind: 'surface', request: req, destination, label: target.label });
      }
      if (!current.signal.aborted) onClose();
    } catch (cause) {
      if (!current.signal.aborted) setSubmitError(errorMessage(cause));
    } finally {
      if (submission.current === current) {
        submission.current = null;
        setSubmitting(false);
      }
    }
  };

  const pageReadability = target?.kind === 'page' ? readabilityHint(previewMinTextPx, previewScale) : null;
  const hasAudioTrack =
    target?.kind === 'map'
      ? audioEnabled
      : target !== null && advancingMapWidgets.some(mw => mw.widgetId === audioMapWidgetId);

  return (
    <dialog
      ref={dialogRef}
      className="export-video-dialog"
      aria-labelledby="export-video-title"
      onCancel={event => {
        event.preventDefault();
        dismiss();
      }}
    >
      {target && (
        <div className="export-video-shell">
          <header className="export-video-header">
            <div>
              <h2 id="export-video-title">Export video — {target.label}</h2>
              <small>Offline constant-frame-rate H.264 MP4</small>
            </div>
            <button type="button" className="icon-button" aria-label="Close video export dialog" onClick={dismiss}>
              <Icon name="close" />
            </button>
          </header>

          <div className="export-video-body">
            <div className="export-video-controls">
              {loading && (
                <p className="export-video-hint">
                  {target.kind === 'map'
                    ? 'Reading map camera and branch timeline…'
                    : 'Preparing presentation layout and embedded maps…'}
                </p>
              )}
              {loadError && <div className="export-video-error">{loadError}</div>}

              {ready && (
                <>
                  <section className="export-video-group">
                    <h3>Time range</h3>
                    <div className="export-video-grid-2">
                      <label className="export-video-field">
                        <span>Start ({formatDateTime(fromHour + bounds.referenceOffset)})</span>
                        <input
                          type="number"
                          min={bounds.firstHour}
                          max={bounds.lastHour}
                          step={24}
                          value={fromHour}
                          onChange={event => setFromHour(Number(event.currentTarget.value))}
                        />
                      </label>
                      <label className="export-video-field">
                        <span>End ({formatDateTime(toHour + bounds.referenceOffset)})</span>
                        <input
                          type="number"
                          min={bounds.firstHour}
                          max={bounds.lastHour}
                          step={24}
                          value={toHour}
                          onChange={event => setToHour(Number(event.currentTarget.value))}
                        />
                      </label>
                    </div>
                    <div className="export-video-row">
                      <button
                        type="button"
                        className="button small quiet"
                        onClick={() => {
                          setFromHour(bounds.firstHour);
                          setToHour(bounds.lastHour);
                        }}
                      >
                        Full branch
                      </button>
                      <button
                        type="button"
                        className="button small quiet"
                        onClick={() => setFromHour(bounds.currentHour)}
                      >
                        Start at current hour
                      </button>
                    </div>
                    <p className="export-video-hint">
                      {coverage.recordedHours.toLocaleString()} recorded hours
                      {coverage.missingHours > 0
                        ? ` · ${coverage.missingHours.toLocaleString()} unrecorded hours across ${coverage.gapCount} gap${coverage.gapCount === 1 ? '' : 's'}`
                        : ' · continuous coverage'}
                    </p>
                  </section>

                  <section className="export-video-group">
                    <h3>Speed and duration</h3>
                    <div className="export-video-grid-2">
                      <label className="export-video-field">
                        <span>Speed (hours / sec)</span>
                        <input
                          type="number"
                          min={0.1}
                          step={1}
                          value={Number(speed.toFixed(2))}
                          onChange={event => {
                            const val = Number(event.currentTarget.value);
                            if (Number.isFinite(val) && val > 0) setSpeed(val);
                          }}
                        />
                      </label>
                      <label className="export-video-field">
                        <span>Duration (seconds)</span>
                        <input
                          type="number"
                          min={0.5}
                          step={1}
                          value={Number(durationSeconds.toFixed(1))}
                          onChange={event => {
                            const val = Number(event.currentTarget.value);
                            if (Number.isFinite(val) && val > 0) {
                              setSpeed(
                                computeSpeedFromDuration(
                                  coverage,
                                  val,
                                  skipGaps,
                                  cutCardSeconds,
                                  fps,
                                  activeStretches ?? undefined,
                                  fromHour,
                                  toHour,
                                ),
                              );
                            }
                          }}
                        />
                      </label>
                    </div>
                    <p className="export-video-hint">
                      Output length: {formatExportDuration(durationSeconds)} ·{' '}
                      {cadence.hoursPerFrame >= 1
                        ? `${cadence.hoursPerFrame.toFixed(2)} hours/frame`
                        : `${cadence.framesPerHour.toFixed(1)} frames/hour`}
                    </p>
                    {cadence.warning && cadence.snappedSpeed && (
                      <div className="export-video-warning">
                        <span>{cadence.warning}</span>
                        <button
                          type="button"
                          className="button small"
                          onClick={() => setSpeed(cadence.snappedSpeed!)}
                        >
                          Snap to {cadence.snappedSpeed} h/s
                        </button>
                      </div>
                    )}
                    {coverage.missingHours > 0 && (
                      <>
                        <label className="export-video-row">
                          <span>Skip unrecorded gaps (insert dated cut cards)</span>
                          <input
                            type="checkbox"
                            role="switch"
                            checked={skipGaps}
                            onChange={event => setSkipGaps(event.currentTarget.checked)}
                          />
                        </label>
                        {skipGaps && (
                          <label className="export-video-field">
                            <span>Cut card duration (seconds)</span>
                            <input
                              type="number"
                              min={0.25}
                              max={5}
                              step={0.25}
                              value={cutCardSeconds}
                              onChange={event => {
                                const val = Number(event.currentTarget.value);
                                if (Number.isFinite(val) && val >= 0.25) setCutCardSeconds(val);
                              }}
                            />
                          </label>
                        )}
                      </>
                    )}
                  </section>

                  <section className="export-video-group">
                    <h3>Dimensions and quality</h3>
                    <label className="export-video-field">
                      <span>Resolution preset</span>
                      <select value={presetId} onChange={event => choosePreset(event.currentTarget.value)}>
                        {DIMENSION_PRESETS.map(item => (
                          <option key={item.id} value={item.id}>
                            {item.label}
                          </option>
                        ))}
                        <option value="match">
                          Match current ({snapEven(bounds.viewport[0])}×{snapEven(bounds.viewport[1])})
                        </option>
                        <option value="custom">Custom</option>
                      </select>
                    </label>
                    <div className="export-video-grid-2">
                      <label className="export-video-field">
                        <span>Width (px)</span>
                        <input
                          type="number"
                          min={16}
                          max={bounds.maxTexture}
                          step={2}
                          value={width}
                          onChange={event => {
                            setPresetId('custom');
                            setWidth(Number(event.currentTarget.value));
                          }}
                          onBlur={() => setWidth(snapEven(width))}
                        />
                      </label>
                      <label className="export-video-field">
                        <span>Height (px)</span>
                        <input
                          type="number"
                          min={16}
                          max={bounds.maxTexture}
                          step={2}
                          value={height}
                          onChange={event => {
                            setPresetId('custom');
                            setHeight(Number(event.currentTarget.value));
                          }}
                          onBlur={() => setHeight(snapEven(height))}
                        />
                      </label>
                    </div>
                    {dimensionError && <div className="export-video-error">{dimensionError}</div>}
                    <div className="export-video-grid-2">
                      <label className="export-video-field">
                        <span>Frame rate</span>
                        <select
                          value={fps}
                          onChange={event => setFps(Number(event.currentTarget.value) === 30 ? 30 : 60)}
                        >
                          <option value={60}>60 fps</option>
                          <option value={30}>30 fps</option>
                        </select>
                      </label>
                      <label className="export-video-field">
                        <span>Quality</span>
                        <select
                          value={quality}
                          onChange={event => setQuality(event.currentTarget.value as Quality)}
                        >
                          <option value="standard">Standard</option>
                          <option value="high">High</option>
                          <option value="maximum">Max</option>
                        </select>
                      </label>
                    </div>
                  </section>

                  {target.kind === 'page' && (
                    <section className="export-video-group">
                      <h3>Page framing</h3>
                      <label className="export-video-field">
                        <span>Layout fit</span>
                        <select
                          value={framing}
                          onChange={event => setFraming(event.currentTarget.value === 'visible' ? 'visible' : 'full')}
                        >
                          <option value="full">Full page (fit entire layout)</option>
                          <option value="visible">Visible viewport (current scroll position)</option>
                        </select>
                      </label>
                      {framing === 'visible' && (
                        <label className="export-video-row">
                          <span>Reflow for portrait</span>
                          <input
                            type="checkbox"
                            role="switch"
                            checked={reflowForPortrait}
                            onChange={event => setReflowForPortrait(event.currentTarget.checked)}
                          />
                        </label>
                      )}
                      {pageReadability && (
                        <div className="export-video-warning">
                          <span>{pageReadability}</span>
                        </div>
                      )}
                    </section>
                  )}

                  <section className="export-video-group">
                    <h3>Overlays and audio</h3>
                    <label className="export-video-row">
                      <span>Corner campaign date stamp</span>
                      <input
                        type="checkbox"
                        role="switch"
                        checked={dateStamp}
                        onChange={event => setDateStamp(event.currentTarget.checked)}
                      />
                    </label>
                    {target.kind === 'map' && (
                      <label className="export-video-row">
                        <span>Include map audio (game and mod audio)</span>
                        <input
                          type="checkbox"
                          role="switch"
                          checked={audioEnabled}
                          onChange={event => {
                            setAudioManual(true);
                            setAudioEnabled(event.currentTarget.checked);
                          }}
                        />
                      </label>
                    )}
                    {target.kind !== 'map' && advancingMapWidgets.length > 0 && (
                      <label className="export-video-field">
                        <span>Map audio source</span>
                        <select
                          value={audioMapWidgetId}
                          onChange={event => {
                            setAudioManual(true);
                            setAudioMapWidgetId(event.currentTarget.value);
                          }}
                        >
                          <option value="">No audio</option>
                          {advancingMapWidgets.map(mw => (
                            <option key={mw.widgetId} value={mw.widgetId}>
                              {mw.title} (game and mod audio)
                            </option>
                          ))}
                        </select>
                      </label>
                    )}
                    {target.kind !== 'map' && target.context.hasPinnedWidgets && (
                      <>
                        <div className="export-video-warning">
                          <span>
                            {target.kind === 'widget'
                              ? 'This widget is pinned to a fixed hour and won’t advance unless switched to follow time.'
                              : 'One or more widgets on this page are pinned to a fixed hour.'}
                          </span>
                        </div>
                        <label className="export-video-row">
                          <span>Follow time for this export</span>
                          <input
                            type="checkbox"
                            role="switch"
                            checked={followPinned}
                            onChange={event => setFollowPinned(event.currentTarget.checked)}
                          />
                        </label>
                      </>
                    )}
                    {target.kind !== 'map' && target.context.hasPagedRowsWidgets && (
                      <p className="export-video-hint">
                        Scrolled log and event tables start at their live head during export.
                      </p>
                    )}
                  </section>
                </>
              )}
            </div>

            <div className="export-video-preview-pane">
              <div className="export-video-preview-toolbar">
                <strong>Framing preview</strong>
                <div className="export-video-footer-actions">
                  {target.kind === 'map' && (
                    <>
                      <button
                        type="button"
                        className="button small quiet"
                        disabled={!mapSnapshot}
                        onClick={() => setCropMode(value => !value)}
                      >
                        {cropMode ? 'View output frame' : 'Adjust crop'}
                      </button>
                      <button
                        type="button"
                        className="button small quiet"
                        disabled={!mapSnapshot}
                        onClick={applyAspectSafeFrame}
                      >
                        Fit aspect
                      </button>
                      {!isFullCrop(crop) && (
                        <button
                          type="button"
                          className="button small quiet"
                          onClick={() => setCrop(FULL_CROP)}
                        >
                          Reset crop
                        </button>
                      )}
                    </>
                  )}
                  <button
                    type="button"
                    className="button small quiet"
                    disabled={!previewUrl || cropMode}
                    onClick={() => setZoomFull(value => !value)}
                  >
                    {zoomFull ? 'Fit preview' : '100% crop'}
                  </button>
                </div>
              </div>

              <div className={`export-video-preview-stage${zoomFull && !cropMode ? ' is-zoomed' : ''}`}>
                {previewUrl ? (
                  <div className="export-video-preview-frame">
                    <img
                      src={previewUrl}
                      alt="Output video frame preview"
                      className="export-video-preview-img"
                      draggable={false}
                    />
                    {target.kind === 'map' && cropMode && (
                      <div className="export-crop-overlay">
                        <div
                          className="export-crop-box"
                          style={{
                            left: `${crop.x * 100}%`,
                            top: `${crop.y * 100}%`,
                            width: `${crop.width * 100}%`,
                            height: `${crop.height * 100}%`,
                          }}
                          onPointerDown={event => startCropDrag('move', event)}
                          onPointerMove={moveCropDrag}
                          onPointerUp={endCropDrag}
                        >
                          <div
                            className="export-crop-handle"
                            onPointerDown={event => startCropDrag('resize', event)}
                            onPointerMove={moveCropDrag}
                            onPointerUp={endCropDrag}
                          />
                        </div>
                      </div>
                    )}
                  </div>
                ) : (
                  <p className="export-video-hint">
                    {previewBusy ? 'Rendering output preview…' : 'Preview will appear here'}
                  </p>
                )}
              </div>

              {previewError && <div className="export-video-error">{previewError}</div>}
              <p className="export-video-hint">
                {target.kind === 'map' && cropMode
                  ? 'Drag the safe frame to crop the camera view, then click “View output frame” to inspect the rendered output.'
                  : `${snapEven(width)}×${snapEven(height)} at ${fps} fps${
                      previewMinTextPx !== null ? ` · min DOM text ~${previewMinTextPx.toFixed(1)} px` : ''
                    } · ${previewBusy ? 'Refreshing preview…' : 'Live output frame'}`}
              </p>
            </div>
          </div>

          {submitError && <div className="export-video-error">{submitError}</div>}
          {busyExport && (
            <div className="export-video-warning">
              <span>Another video export is already running. Wait for it to finish or cancel it first.</span>
            </div>
          )}

          <footer className="export-video-footer">
            <span className="export-video-footer-meta">
              {ready
                ? `${estimatedFrames.toLocaleString()} frames · H.264 MP4${hasAudioTrack ? ' + AAC' : ''}`
                : 'Preparing…'}
            </span>
            <div className="export-video-footer-actions">
              <button type="button" className="button quiet" onClick={dismiss}>
                Cancel
              </button>
              <button
                type="button"
                className="button primary"
                disabled={!ready || !!dimensionError || submitting || busyExport}
                onClick={() => void handleExport()}
              >
                <Icon name="save" />
                {submitting ? 'Starting…' : 'Export MP4…'}
              </button>
            </div>
          </footer>
        </div>
      )}
    </dialog>
  );
}
