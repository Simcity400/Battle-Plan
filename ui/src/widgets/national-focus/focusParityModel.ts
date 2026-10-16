import type { Artwork, FocusEntry, FocusOrder } from '../../generated/dashboard';
import type { FocusProgressState } from './focusData';
import type { ParityList } from '../../dashboard/parityRows';

export interface FocusParityPresentation {
  artwork: Artwork | null;
  pending: boolean;
  reason: string | null;
  descriptionDemanded: boolean;
}
/** The oracle carries metadata for future tokens too. Only referenced tokens
 * affect this shown-hour widget; every field for those tokens stays observable. */
export function focusParityModel(order: FocusOrder | null, progress: FocusProgressState | null,
  readingHour: number | undefined, currentHour: number, moment: boolean, staticText: readonly Artwork[],
  at: (token: string, hour: number) => FocusParityPresentation): ParityList {
  const completed = order?.completed ?? [];
  const tokens = new Set(completed.map(entry => entry.token));
  if (order?.current) tokens.add(order.current);
  if (order?.continuous) tokens.add(order.continuous);
  const referencedStaticText = staticText.filter(item => tokens.has(item.token)).slice().sort((a, b) => a.token < b.token ? -1 : a.token > b.token ? 1 : 0);
  const unfinished = moment ? null : order?.current;
  const rows = completed.map((entry: FocusEntry, index) => ({ key: String(entry.ordinal), index,
    item: { entry, readHour: entry.completedByHour ?? readingHour } }));
  const artwork = completed.map(entry => ({ key: String(entry.ordinal), readHour: entry.completedByHour ?? readingHour,
    art: at(entry.token, entry.completedByHour ?? readingHour!) }));
  const active = order?.current ?? order?.continuous;
  return { kind: 'windowed', label: 'Completed focuses in order', complete: true,
    total: completed.length + (unfinished ? 1 : 0),
    current: order && { current: order.current, continuous: order.continuous, paused: order.paused, progress },
    staticText: referencedStaticText,
    rows: [...rows, ...(unfinished ? [{ key: String(completed.length), index: completed.length,
      item: { token: unfinished, readHour: currentHour, paused: order?.paused, unfinished: true } }] : [])],
    artwork, activeArt: active ? at(active, currentHour) : null };
}
