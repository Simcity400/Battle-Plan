import { Icon } from './Icons';
import { Segmented } from './Segmented';
import type { CameraMode } from '../generated/dashboard';

export const CAMERA_MODES = [
  { value: 'map', label: 'Map', icon: 'map', tooltip: 'The game’s map view.' },
  { value: 'fly', label: 'Fly', icon: 'plane', tooltip: 'Fly over the map as a free camera, or pilot the aircraft you choose. Click the map to start; Esc gives the pointer back and shows the controls.' },
  { value: 'walk', label: 'Walk', icon: 'walk', tooltip: 'Walk the map as your country’s infantry, at any size. Click the map to start; Esc gives the pointer back and shows the controls.' },
] as const satisfies readonly { value: CameraMode; label: string; icon: 'map' | 'plane' | 'walk'; tooltip: string }[];

export const OVERLAYS_TOOLTIP = 'Show the map’s counters, names, borders and plans while flying or walking.';

export default function CameraModeControl({ value, onChange, overlays, onOverlaysChange }: {
  value: CameraMode; onChange: (mode: CameraMode) => void; overlays: boolean; onOverlaysChange: (shown: boolean) => void;
}) {
  return <>
    <Segmented label="Camera" value={value} onChange={onChange} options={CAMERA_MODES} />
    {value !== 'map' && <div className="segmented" role="group" aria-label="Camera overlays">
      <button type="button" aria-pressed={overlays} data-tooltip={OVERLAYS_TOOLTIP} onClick={() => onOverlaysChange(!overlays)}>
        <Icon name="eye" />Overlays</button>
    </div>}
  </>;
}
