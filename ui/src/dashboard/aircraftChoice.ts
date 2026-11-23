import type { AircraftEntry } from '../generated/dashboard';

const PREFIX = 'battleplan:fly-aircraft:';

type ChoiceStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export interface AircraftChoice { name: string; title: string }

export function modelLabel(name: string): string {
  const words = name.replace(/(?:[_-](?:entity|mesh))+$/i, '').replace(/[_\-.]+/g, ' ').trim().replace(/\s+/g, ' ');
  return words ? words[0].toUpperCase() + words.slice(1) : 'Model';
}

export function aircraftTitle(entry: Pick<AircraftEntry, 'titles' | 'name'>): string {
  return entry.titles[0] ?? modelLabel(entry.name);
}

export function loadAircraftChoice(recording: string, storage?: ChoiceStorage): AircraftChoice | null {
  try {
    const raw = (storage ?? localStorage).getItem(PREFIX + recording);
    if (!raw) return null;
    if (!raw.startsWith('{')) return { name: raw, title: modelLabel(raw) };
    const stored: unknown = JSON.parse(raw);
    if (typeof stored !== 'object' || stored === null) return null;
    const { name, title } = stored as Record<string, unknown>;
    return typeof name === 'string' && name ? { name, title: typeof title === 'string' && title ? title : modelLabel(name) } : null;
  } catch { return null; }
}

export function saveAircraftChoice(recording: string, aircraft: AircraftChoice | null, storage?: ChoiceStorage): void {
  try {
    const target = storage ?? localStorage;
    if (aircraft) target.setItem(PREFIX + recording, JSON.stringify({ name: aircraft.name, title: aircraft.title }));
    else target.removeItem(PREFIX + recording);
  } catch {   }
}
