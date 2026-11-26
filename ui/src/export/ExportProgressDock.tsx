import { useLayoutEffect, useRef, useState } from 'react';
import { Icon } from '../components/Icons';
import { formatDateTime } from '../dashboard/calendar';
import { errorMessage } from '../shell/api';
import {
  cancelVideoExport,
  dismissVideoExport,
  revealExportFile,
  savePartialVideoExport,
  type ExportJobSnapshot,
} from './videoExportApi';
import { formatExportDuration } from './videoExportModel';
import './exportVideo.css';

export default function ExportProgressDock({
  job,
  onRetry,
}: {
  job: ExportJobSnapshot;
  onRetry?: () => void;
}) {
  const dockRef = useRef<HTMLDivElement>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [savingPartial, setSavingPartial] = useState(false);
  const visible = job.state !== 'idle';

  useLayoutEffect(() => {
    const element = dockRef.current;
    if (!element) return;
    if (visible) {
      if (!element.matches(':popover-open')) {
        try {
          element.showPopover();
        } catch {
        }
      }
    } else if (element.matches(':popover-open')) {
      try {
        element.hidePopover();
      } catch {
      }
    }
  }, [visible]);

  if (!visible) {
    return <div ref={dockRef} popover="manual" className="export-progress-dock" hidden aria-hidden="true" />;
  }

  const ratio = job.totalFrames > 0 ? Math.min(100, Math.round((job.frame / job.totalFrames) * 100)) : 0;
  const heading =
    job.state === 'running'
      ? `Exporting ${job.targetLabel || 'video'}…`
      : job.state === 'completed'
        ? `Exported ${job.targetLabel || 'video'}`
        : job.state === 'cancelled'
          ? 'Video export cancelled'
          : 'Video export failed';

  const handleSavePartial = async () => {
    setSavingPartial(true);
    setActionError(null);
    try {
      await savePartialVideoExport(job.id);
    } catch (cause) {
      setActionError(errorMessage(cause));
    } finally {
      setSavingPartial(false);
    }
  };

  const handleReveal = async () => {
    if (!job.destination) return;
    setActionError(null);
    try {
      await revealExportFile(job.destination);
    } catch (cause) {
      setActionError(errorMessage(cause));
    }
  };

  return (
    <div ref={dockRef} popover="manual" className="export-progress-dock" role="status" aria-live="polite">
      <div className="export-progress-head">
        <strong>{heading}</strong>
        {job.backend && <small className="export-video-hint">{job.backend}</small>}
      </div>

      {job.state === 'running' && (
        <>
          <div className="export-progress-bar" aria-hidden="true">
            <div className="export-progress-bar-fill" style={{ width: `${ratio}%` }} />
          </div>
          <div className="export-progress-meta">
            <span>
              {job.campaignHour != null ? formatDateTime(job.campaignHour) : job.phase}
            </span>
            <span>
              {job.totalFrames > 0 ? `${job.frame} / ${job.totalFrames} (${ratio}%)` : job.phase}
            </span>
          </div>
          <div className="export-progress-meta">
            <span>Elapsed {formatExportDuration(job.elapsedMs / 1000)}</span>
            <span>{job.etaMs != null ? `ETA ${formatExportDuration(job.etaMs / 1000)}` : 'Estimating…'}</span>
          </div>
        </>
      )}

      {job.state === 'completed' && job.destination && (
        <p className="export-video-hint">{job.destination}</p>
      )}

      {job.warnings.length > 0 && (
        <p className="export-video-hint">{job.warnings.join(' ')}</p>
      )}

      {job.error && <div className="export-video-error">{job.error}</div>}
      {actionError && <div className="export-video-error">{actionError}</div>}

      <div className="export-progress-actions">
        {job.state === 'running' && (
          <button type="button" className="button small" onClick={() => void cancelVideoExport()}>
            <Icon name="close" />Cancel
          </button>
        )}
        {job.state === 'completed' && job.destination && (
          <button type="button" className="button small primary" onClick={() => void handleReveal()}>
            <Icon name="folder-open" />Open folder
          </button>
        )}
        {job.state === 'failed' && job.partial && (
          <button
            type="button"
            className="button small primary"
            disabled={savingPartial}
            onClick={() => void handleSavePartial()}
          >
            <Icon name="save" />
            Save partial through {formatDateTime(job.partial.throughHour)}
          </button>
        )}
        {job.state === 'failed' && onRetry && (
          <button type="button" className="button small" onClick={onRetry}>
            <Icon name="refresh" />Retry
          </button>
        )}
        {job.state !== 'running' && (
          <button type="button" className="button small quiet" onClick={() => void dismissVideoExport()}>
            Dismiss
          </button>
        )}
      </div>
    </div>
  );
}
