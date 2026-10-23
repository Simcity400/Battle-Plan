/** The aircraft Fly mode flies, kept per recording on this device: a name of that recording's content, so another recording starts with none. There is no default. */

const PREFIX = 'battle-plan:fly-aircraft:';

type ChoiceStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

/** The aircraft chosen for the recording `recordingKey` names, or none; storage that cannot be read keeps none. */
export function loadAircraftChoice(recording: string, storage?: ChoiceStorage): string | null {
  try {
    const name = (storage ?? localStorage).getItem(PREFIX + recording);
    return name ? name : null;
  } catch { return null; }
}

/** Keeps or forgets the recording's choice; whether this device keeps it changes nothing about the map. */
export function saveAircraftChoice(recording: string, aircraft: string | null, storage?: ChoiceStorage): void {
  try {
    const target = storage ?? localStorage;
    if (aircraft) target.setItem(PREFIX + recording, aircraft);
    else target.removeItem(PREFIX + recording);
  } catch { /* The choice still applies for this session. */ }
}
