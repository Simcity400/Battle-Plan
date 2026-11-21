export interface MapSwitch { value: boolean; sequence: number }

export function newestSwitch(show: (value: boolean) => void): (next: MapSwitch) => boolean {
  let newest = -1;
  return next => {
    if (next.sequence < newest) return false;
    newest = next.sequence;
    show(next.value);
    return true;
  };
}
