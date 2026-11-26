import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent } from 'react';
import { Icon } from '../components/Icons';
import { formatDateTime } from '../dashboard/calendar';
import { errorMessage } from '../shell/api';
import {
  chooseVideoExportPath,
  getMapExportSnapshot,
  previewMapVideo,
  startMapVideoExport,
  type CropRect,
  type MapExportSnapshot,
  type MapVideoRequest,
  type Quality,
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
  normalizeCrop,
  snapEven,
  summarizeRange,
  validateDimensions,
} from './videoExportModel';
import type { MapVideoExportTarget } from './ExportVideoContext';
import './exportVideo.css';

export default function ExportVideoDialog({
  target,
  busyExport,
  onClose,
  onStarted,
}: {
  target: MapVideoExportTarget | null;
  busyExport: boolean;
  onClose: () => void;
  onStarted: (request: MapVideoRequest, destination: string, label: string) => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [snapshot, setSnapshot] = useState<MapExportSnapshot | null>(null);
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
  const [crop, setCrop] = useState<CropRect>(FULL_CROP);
  const [cropMode, setCropMode] = useState(false);
  const [zoomFull, setZoomFull] = useState(false);

  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const dragRef = useRef<{
    kind: 'move' | 'resize';
    startX: number;
    startY: number;
    startCrop: CropRect;
    rect: DOMRect;
  } | null>(null);

  useLayoutEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (target) {
      if (!dialog.open) dialog.showModal();
    } else if (dialog.open) {
      dialog.close();
    }
  }, [target]);

  useEffect(() => {
    if (!target) {
      setSnapshot(null);
      setLoadError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    setSubmitError(null);
    setPreviewError(null);
    setAudioManual(false);
    setCrop(FULL_CROP);
    setCropMode(false);
    setZoomFull(false);
    void getMapExportSnapshot(target.viewId)
      .then(snap => {
        if (cancelled) return;
        setSnapshot(snap);
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
    return () => {
      cancelled = true;
    };
  }, [target]);

  useEffect(() => {
    if (!audioManual && snapshot) {
      setAudioEnabled(defaultAudioForSpeed(speed) && !snapshot.audioMuted);
    }
  }, [speed, audioManual, snapshot]);

  const previewGenRef = useRef(0);

  const coverage = useMemo(() => {
    if (!snapshot) return { spanHours: 0, recordedHours: 0, missingHours: 0, gapCount: 0 };
    return summarizeRange(snapshot.stretches, snapshot.unheld, fromHour, toHour);
  }, [snapshot, fromHour, toHour]);

  const estimatedFrames = useMemo(() => {
    if (!snapshot) return 0;
    return estimateScheduleFrames(snapshot.stretches, fromHour, toHour, fps, speed, skipGaps, cutCardSeconds);
  }, [snapshot, fromHour, toHour, fps, speed, skipGaps, cutCardSeconds]);

  const durationSeconds = useMemo(
    () =>
      computeDurationSeconds(
        coverage,
        speed,
        skipGaps,
        cutCardSeconds,
        fps,
        snapshot?.stretches,
        fromHour,
        toHour,
      ),
    [coverage, speed, skipGaps, cutCardSeconds, fps, snapshot, fromHour, toHour],
  );

  const cadence = useMemo(() => analyzeCadence(speed, fps), [speed, fps]);

  const dimensionError = useMemo(
    () => validateDimensions(width, height, snapshot?.maxTextureDimension ?? 8192),
    [width, height, snapshot],
  );

  const buildRequest = useCallback(
    (useFullFrameForCropEdit: boolean): MapVideoRequest | null => {
      if (!snapshot) return null;
      const evenW = snapEven(width);
      const evenH = snapEven(height);
      let previewW = evenW;
      let previewH = evenH;
      if (useFullFrameForCropEdit) {
        const srcW = Math.max(16, snapshot.viewport[0] || 1280);
        const srcH = Math.max(16, snapshot.viewport[1] || 720);
        const factor = Math.min(1, 1280 / srcW, 720 / srcH);
        previewW = snapEven(srcW * factor);
        previewH = snapEven(srcH * factor);
      }
      return {
        path: snapshot.path,
        revision: snapshot.revision,
        branch: snapshot.branch,
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
        placement: snapshot.placement,
        sourceViewport: snapshot.viewport,
        crop: useFullFrameForCropEdit || isFullCrop(crop) ? null : normalizeCrop(crop),
        uiScale: snapshot.uiScale,
        mapMode: snapshot.mapMode,
        perspective: snapshot.perspective,
        perspectiveSlot: snapshot.perspectiveSlot ?? null,
        fog: snapshot.fog,
        dayNight: snapshot.dayNight,
        alliedPlans: snapshot.alliedPlans,
        counterColorsCountry: snapshot.counterColorsCountry,
        natoCounters: snapshot.natoCounters,
        mapSettings: snapshot.mapSettings,
        dateStamp,
        audioEnabled,
        allowUnverified: false,
      };
    },
    [
      snapshot,
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

  const renderPreview = useCallback(async () => {
    if (!snapshot || dimensionError) return;
    const req = buildRequest(cropMode);
    if (!req) return;
    const gen = ++previewGenRef.current;
    setPreviewBusy(true);
    setPreviewError(null);
    try {
      const previewHour = Math.min(Math.max(snapshot.currentHour, req.fromHour), req.toHour);
      const pngBytes = await previewMapVideo(req, previewHour);
      if (gen !== previewGenRef.current) return;
      let binary = '';
      const chunk = 0x8000;
      for (let i = 0; i < pngBytes.length; i += chunk) {
        binary += String.fromCharCode(...pngBytes.subarray(i, i + chunk));
      }
      setPreviewUrl(`data:image/png;base64,${btoa(binary)}`);
    } catch (cause) {
      if (gen !== previewGenRef.current) return;
      setPreviewError(errorMessage(cause));
    } finally {
      if (gen === previewGenRef.current) {
        setPreviewBusy(false);
      }
    }
  }, [snapshot, dimensionError, buildRequest, cropMode]);

  useEffect(() => {
    previewGenRef.current += 1;
    setPreviewUrl(null);
    setPreviewError(null);
    setPreviewBusy(false);
  }, [target]);

  useEffect(() => {
    if (!snapshot || !target || dimensionError) return;
    const timer = window.setTimeout(() => {
      void renderPreview();
    }, 220);
    return () => window.clearTimeout(timer);
  }, [snapshot, target, dimensionError, renderPreview]);

  const choosePreset = (id: string) => {
    setPresetId(id);
    if (id === 'match' && snapshot) {
      setWidth(snapEven(snapshot.viewport[0] || 1920));
      setHeight(snapEven(snapshot.viewport[1] || 1080));
      return;
    }
    const found = DIMENSION_PRESETS.find(item => item.id === id);
    if (found) {
      setWidth(found.width);
      setHeight(found.height);
    }
  };

  const applyAspectSafeFrame = () => {
    if (!snapshot) return;
    setCrop(fitAspectCrop(snapshot.viewport, width, height));
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
    if (!target || !snapshot || dimensionError || busyExport) return;
    const req = buildRequest(false);
    if (!req) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const safeName = target.label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'campaign-map';
      const destination = await chooseVideoExportPath(`${safeName}.mp4`);
      if (!destination) return;
      await startMapVideoExport(req, destination, target.label);
      onStarted(req, destination, target.label);
      onClose();
    } catch (cause) {
      setSubmitError(errorMessage(cause));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <dialog
      ref={dialogRef}
      className="export-video-dialog"
      aria-labelledby="export-video-title"
      onCancel={event => {
        event.preventDefault();
        onClose();
      }}
    >
      {target && (
        <div className="export-video-shell">
          <header className="export-video-header">
            <div>
              <h2 id="export-video-title">Export video — {target.label}</h2>
              <small>Offline constant-frame-rate H.264 MP4</small>
            </div>
            <button type="button" className="icon-button" aria-label="Close video export dialog" onClick={onClose}>
              <Icon name="close" />
            </button>
          </header>

          <div className="export-video-body">
            <div className="export-video-controls">
              {loading && <p className="export-video-hint">Reading map camera and branch timeline…</p>}
              {loadError && <div className="export-video-error">{loadError}</div>}

              {snapshot && (
                <>
                  <section className="export-video-group">
                    <h3>Time range</h3>
                    <div className="export-video-grid-2">
                      <label className="export-video-field">
                        <span>Start ({formatDateTime(fromHour)})</span>
                        <input
                          type="number"
                          min={snapshot.firstHour}
                          max={snapshot.lastHour}
                          step={24}
                          value={fromHour}
                          onChange={event => setFromHour(Number(event.currentTarget.value))}
                        />
                      </label>
                      <label className="export-video-field">
                        <span>End ({formatDateTime(toHour)})</span>
                        <input
                          type="number"
                          min={snapshot.firstHour}
                          max={snapshot.lastHour}
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
                          setFromHour(snapshot.firstHour);
                          setToHour(snapshot.lastHour);
                        }}
                      >
                        Full branch
                      </button>
                      <button
                        type="button"
                        className="button small quiet"
                        onClick={() => setFromHour(snapshot.currentHour)}
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
                              setSpeed(computeSpeedFromDuration(coverage, val, skipGaps, cutCardSeconds));
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
                      <select
                        value={presetId}
                        onChange={event => choosePreset(event.currentTarget.value)}
                      >
                        {DIMENSION_PRESETS.map(item => (
                          <option key={item.id} value={item.id}>
                            {item.label}
                          </option>
                        ))}
                        <option value="match">
                          Match current ({snapEven(snapshot.viewport[0])}×{snapEven(snapshot.viewport[1])})
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
                          max={snapshot.maxTextureDimension}
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
                          max={snapshot.maxTextureDimension}
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
                  </section>
                </>
              )}
            </div>

            <div className="export-video-preview-pane">
              <div className="export-video-preview-toolbar">
                <strong>Framing preview</strong>
                <div className="export-video-footer-actions">
                  <button
                    type="button"
                    className="button small quiet"
                    disabled={!snapshot}
                    onClick={() => setCropMode(value => !value)}
                  >
                    {cropMode ? 'View output frame' : 'Adjust crop'}
                  </button>
                  <button
                    type="button"
                    className="button small quiet"
                    disabled={!snapshot}
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
                    {cropMode && (
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
                {cropMode
                  ? 'Drag the safe frame to crop the camera view, then click “View output frame” to inspect the rendered output.'
                  : `${snapEven(width)}×${snapEven(height)} at ${fps} fps · ${previewBusy ? 'Refreshing preview…' : 'Live output frame'}`}
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
              {snapshot
                ? `${estimatedFrames.toLocaleString()} frames · H.264 MP4${audioEnabled ? ' + AAC' : ''}`
                : 'Preparing…'}
            </span>
            <div className="export-video-footer-actions">
              <button type="button" className="button quiet" onClick={onClose}>
                Cancel
              </button>
              <button
                type="button"
                className="button primary"
                disabled={!snapshot || !!dimensionError || submitting || busyExport}
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
