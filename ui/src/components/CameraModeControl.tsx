import { Segmented } from './Segmented';
import type { CameraMode } from '../generated/dashboard';

/** The full map's camera modes, in the order the control shows them. */
export const CAMERA_MODES = [
  { value: 'map', label: 'Map', icon: 'map', tooltip: "The game's map camera. A controller moves, turns, tilts and zooms it" },
  { value: 'fly', label: 'Fly', icon: 'plane', tooltip: 'Fly over the map. Click the map to look with the mouse; Escape gives the pointer back' },
  { value: 'walk', label: 'Walk', icon: 'walk', tooltip: 'Explore the map from the ground. Click the map to look with the mouse; Escape gives the pointer back' },
] as const satisfies readonly { value: CameraMode; label: string; icon: 'map' | 'plane' | 'walk'; tooltip: string }[];

/** Chooses how the full map's camera moves. */
export default function CameraModeControl({ value, onChange }: { value: CameraMode; onChange: (mode: CameraMode) => void }) {
  return <Segmented label="Camera" value={value} onChange={onChange} options={CAMERA_MODES} />;
}
