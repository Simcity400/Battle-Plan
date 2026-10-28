/**
 * One of the maps' switches (mute, diagnostics) as the desktop host holds it. The host numbers
 * its changes to the switches, and `sequence` is the last change `value` reflects.
 */
export interface MapSwitch { value: boolean; sequence: number }

/**
 * Shows a switch's values from its events, its read and its setter's replies, which arrive in
 * any order: only one at least as new as every value shown before. Returns whether it showed it.
 */
export function newestSwitch(show: (value: boolean) => void): (next: MapSwitch) => boolean {
  let newest = -1;
  return next => {
    if (next.sequence < newest) return false;
    newest = next.sequence;
    show(next.value);
    return true;
  };
}
