import { useState } from 'react';
import { Icon } from '../../components/Icons';
import { MapHistorySection } from '../../components/MapControls';
import MapAppearance from '../../components/MapAppearance';
import { FailureLine } from '../../components/FailureLine';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { actionFailure, type PlainFailure } from '../../dashboard/queryError.ts';
import { desktopAvailable, errorMessage, exportMap } from '../../shell/api';
import type { MapOptions, SavedExport } from '../../shell/api';
import { useExportVideo } from '../../export/ExportVideoContext';
import '../../components/mapControls.css';

export function MapSettingsSection({ widget, reference, runtime, mapOptions, onMapOptions }: {
  widget: WidgetSpec;
  reference: BoundTrack | undefined;
  runtime: WidgetRuntime;
  mapOptions: MapOptions;
  onMapOptions: (options: MapOptions) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<(PlainFailure & { retry?: () => void }) | null>(null);
  const [saved, setSaved] = useState<SavedExport | null>(null);
  const { openMapVideoExport } = useExportVideo();
  const metadata = reference?.source.metadata ?? null;
  const branch = reference?.branch ?? null;

  const exportImage = async () => {
    setBusy(true); setError(null); setSaved(null);
    try {
      await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      const answer = await exportMap(widget.id);
      setSaved(answer);
    } catch (cause) {
      setError({ ...actionFailure('Couldn’t export the map.', errorMessage(cause)), retry: () => void exportImage() });
    } finally {
      setBusy(false);
    }
  };

  return <div className="map-settings-section">
    <MapHistorySection metadata={metadata} branch={branch} busy={busy}
      onBranch={id => { if (reference) runtime.onMapBranch(reference.id, id); }} />
    <div className="map-menu-section">
      <button type="button" className="map-menu-action" disabled={!desktopAvailable || !metadata || busy}
        data-tooltip={desktopAvailable ? 'Save the rendered map as a PNG image' : 'Available in the desktop app'} onClick={() => void exportImage()}>
        <Icon name="save" />Export map image
      </button>
      <button type="button" className="map-menu-action" disabled={!desktopAvailable || !metadata || busy}
        data-tooltip={desktopAvailable ? 'Export a constant-frame-rate MP4 video of this map widget' : 'Available in the desktop app'}
        onClick={() => openMapVideoExport({ viewId: widget.id, label: widget.title || 'Map widget' })}>
        <Icon name="play" />Export map video…
      </button>
      {saved && <p className="map-menu-note" role="status" data-detail={saved.warning ?? undefined}>
        {saved.warning ? `${saved.message}, but Windows didn’t confirm it reached the disk.` : saved.message}
      </p>}
      {error && <FailureLine className="map-menu-error" message={error.text} detail={error.detail} onRetry={error.retry} retryDisabled={busy} />}
    </div>
    <div className="map-menu-section">
      <label className="map-option-row"><span>Fog of war</span>
        <input type="checkbox" role="switch" disabled={!reference} checked={mapOptions.fog}
          onChange={event => onMapOptions({ ...mapOptions, fog: event.currentTarget.checked })} />
      </label>
      {!desktopAvailable && <p className="map-menu-note">Map display options apply in the desktop app.</p>}
    </div>
    <MapAppearance key={reference ? `${reference.source.id}:${reference.source.revision}` : 'empty'} source={reference?.source ?? null} client={runtime.client} viewId={widget.id} />
  </div>;
}
