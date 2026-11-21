import MapControls from '../../components/MapControls';
import { Icon } from '../../components/Icons';
import { resolveWidgetCampaign } from '../../dashboard/campaigns';
import { WidgetSettings } from '../../dashboard/settings/WidgetSettings';
import type { HeaderControlsProps } from '../../dashboard/definitions';

export default function MapWidgetControls({ widget, runtime, template, editing, onApply, onRemove, onReset, mapOptions, onMapOptions }: HeaderControlsProps) {
  const resolved = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const track = runtime.tracks.find(item => item.id === resolved.trackIds[0]);
  return <MapControls compact viewId={widget.id} source={track?.source ?? null} client={runtime.client} metadata={track?.source.metadata ?? null}
    branchId={track?.branch.id ?? null} onBranch={id => { if (track) runtime.onMapBranch(track.id, id); }} options={mapOptions} onOptions={onMapOptions}
    onRemove={editing ? onRemove : undefined} onReset={onReset}>
    <details className="map-menu-section map-widget-options"><summary><Icon name="sliders" />Widget settings</summary>
      <WidgetSettings widget={widget} template={template} runtime={runtime} onChange={onApply} />
    </details>
  </MapControls>;
}
