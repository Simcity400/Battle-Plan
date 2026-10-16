import { Icon } from './Icons';
import { Segmented } from './Segmented';
import type { CameraMode } from '../generated/dashboard';

/** The full map's camera modes, in the order the control shows them. */
export const CAMERA_MODES = [
  { value: 'map', label: 'Map', icon: 'map', tooltip: "The game's map camera. A controller moves, turns, tilts and zooms it" },
  { value: 'fly', label: 'Fly', icon: 'plane', tooltip: 'Fly over the map. Click the map to look with the mouse; Escape gives the pointer back' },
  { value: 'walk', label: 'Walk', icon: 'walk', tooltip: "Walk the map as your country's infantry. Click the map to look with the mouse; W A S D walk, Space twice flies, hold + or - to grow or shrink; Escape gives the pointer back" },
] as const satisfies readonly { value: CameraMode; label: string; icon: 'map' | 'plane' | 'walk'; tooltip: string }[];

/** What the fly and walk modes' overlay switch does. */
export const OVERLAYS_TOOLTIP = "Show the map's counters, icons, names, borders and plans while flying or walking. They stand in the world, fading behind hills and with distance";

/** Chooses how the full map's camera moves and, flying or walking, whether the map's interface shows. */
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
