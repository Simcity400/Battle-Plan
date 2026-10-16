/** The aircraft Fly mode flies, kept per recording on this device: a name of that recording's content, so another recording starts with none. There is no default. */

const PREFIX = 'battle-plan:fly-aircraft:';

type ChoiceStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

/** The aircraft chosen for the recording at `path`, or none; storage that cannot be read keeps none. */
export function loadAircraftChoice(path: string, storage?: ChoiceStorage): string | null {
  try {
    const name = (storage ?? localStorage).getItem(PREFIX + path);
    return name ? name : null;
  } catch { return null; }
}

/** Keeps or forgets the recording's choice; whether this device keeps it changes nothing about the map. */
export function saveAircraftChoice(path: string, aircraft: string | null, storage?: ChoiceStorage): void {
  try {
    const target = storage ?? localStorage;
    if (aircraft) target.setItem(PREFIX + path, aircraft);
    else target.removeItem(PREFIX + path);
  } catch { /* The choice still applies for this session. */ }
}
