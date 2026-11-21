
const PREFIX = 'battle-plan:fly-aircraft:';

type ChoiceStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export function loadAircraftChoice(recording: string, storage?: ChoiceStorage): string | null {
  try {
    const name = (storage ?? localStorage).getItem(PREFIX + recording);
    return name ? name : null;
  } catch { return null; }
}

export function saveAircraftChoice(recording: string, aircraft: string | null, storage?: ChoiceStorage): void {
  try {
    const target = storage ?? localStorage;
    if (aircraft) target.setItem(PREFIX + recording, aircraft);
    else target.removeItem(PREFIX + recording);
  } catch {   }
}
