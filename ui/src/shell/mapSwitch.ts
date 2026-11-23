export interface MapSwitch { value: boolean; sequence: number; warning: string | null }

export function newestSwitch(show: (next: MapSwitch) => void): (next: MapSwitch) => boolean {
  let newest = -1;
  return next => {
    if (next.sequence <= newest) return false;
    newest = next.sequence;
    show(next);
    return true;
  };
}
