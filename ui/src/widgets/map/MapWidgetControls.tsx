import MapControls from '../../components/MapControls';
import { resolveWidgetCampaign } from '../../dashboard/campaigns';
import { WidgetSettings } from '../../dashboard/controls';
import type { HeaderControlsProps } from '../../dashboard/definitions';

/** The map's header pop-down: appearance, branch, export and the shared widget settings. */
export default function MapWidgetControls({ widget, runtime, template, editing, onApply, onRemove, onReset, mapOptions, onMapOptions }: HeaderControlsProps) {
  const resolved = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const track = runtime.tracks.find(item => item.id === resolved.trackIds[0]);
  return <MapControls compact viewId={widget.id} source={track?.source ?? null} client={runtime.client} metadata={track?.source.metadata ?? null}
    branchId={track?.branch.id ?? null} onBranch={id => { if (track) runtime.onMapBranch(track.id, id); }} options={mapOptions} onOptions={onMapOptions}
    onRemove={editing ? onRemove : undefined} onReset={onReset}>
    <details className="map-menu-section map-widget-options"><summary>Widget settings</summary>
      <WidgetSettings widget={widget} template={template} tracks={runtime.tracks} client={runtime.client} defaultRoleId={runtime.defaultRoleId} onChange={onApply} />
    </details>
  </MapControls>;
}
