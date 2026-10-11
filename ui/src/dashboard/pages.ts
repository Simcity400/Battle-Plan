// Pure page metadata for the sidebar and the layout model; no React or CSS.
import type { IconName } from '../components/Icons';
import type { WidgetKind } from './model';

/** A default widget: kind, then x, y, width and height on the 12-column grid, in reading order. */
type Panel = readonly [WidgetKind, number, number, number, number];

/** Analytics pages. Each keeps its own saved layouts and starts from this arrangement. */
export const PAGES = [
  { id: 'overview', label: 'Overview', icon: 'grid', panels: [
    ['map', 0, 0, 8, 14], ['event-feed', 8, 0, 4, 14],
    ['scoreboard', 0, 14, 8, 10], ['game-score', 8, 14, 4, 10],
    ['territory', 0, 24, 4, 10], ['faction-strength', 4, 24, 4, 10], ['world-tension', 8, 24, 4, 10],
    ['wars', 0, 34, 12, 9],
  ] },
  { id: 'politics', label: 'Politics', icon: 'parliament', panels: [
    ['national-focus', 0, 0, 4, 16], ['ruling-party', 4, 0, 8, 8], ['stability', 4, 8, 8, 8],
    ['political-power', 0, 16, 6, 10], ['political-power-spending-breakdown', 6, 16, 6, 10],
    ['political-power-spending', 0, 26, 12, 10],
    ['command-power', 0, 36, 6, 10], ['command-power-spending', 6, 36, 6, 10],
    ['national-spirits', 0, 46, 4, 8], ['spirit-history', 4, 46, 8, 8],
    ['idea-timeline', 0, 54, 12, 9],
  ] },
  { id: 'diplomacy', label: 'Diplomacy', icon: 'handshake', panels: [
    ['factions', 0, 0, 6, 10], ['subjects', 6, 0, 6, 10],
    ['wars', 0, 10, 12, 10],
    ['world-tension', 0, 20, 6, 10], ['autonomy', 6, 20, 6, 10],
    ['peace-deals', 0, 30, 6, 10], ['territory-transfers', 6, 30, 6, 10],
  ] },
  { id: 'economy', label: 'Economy', icon: 'factory', panels: [
    ['factories', 0, 0, 6, 10], ['equipment-produced', 6, 0, 6, 10],
    ['production-lines', 0, 10, 12, 11],
    ['production-spending', 0, 21, 6, 10], ['equipment-stockpile', 6, 21, 6, 10],
    ['resource-balance', 0, 31, 6, 11], ['trade-flows', 6, 31, 6, 11],
    ['fuel', 0, 42, 4, 10], ['construction-spending', 4, 42, 8, 10],
    ['construction-log', 0, 52, 12, 10],
  ] },
  { id: 'research', label: 'Research', icon: 'flask', panels: [
    ['research', 0, 0, 7, 18], ['special-projects', 7, 0, 5, 10],
    ['nuclear-arsenal', 7, 10, 5, 8],
  ] },
  { id: 'land', label: 'Army', icon: 'helmet', panels: [
    ['army-size', 0, 0, 6, 10], ['casualties', 6, 0, 6, 10],
    ['land-battles', 0, 10, 12, 11],
    ['casualties-between', 0, 21, 6, 10], ['supply-health', 6, 21, 6, 10],
    ['generals', 0, 31, 7, 12], ['general-career', 7, 31, 5, 12],
    ['casualties-breakdown', 0, 43, 6, 10], ['equipment-losses', 6, 43, 6, 10],
    ['division-log', 0, 53, 8, 10], ['experience', 8, 53, 4, 10],
  ] },
  { id: 'air', label: 'Air', icon: 'plane', panels: [
    ['air-force-size', 0, 0, 6, 10], ['air-kills-losses', 6, 0, 6, 10],
    ['air-superiority-region', 0, 10, 6, 11], ['air-superiority-timeline', 6, 10, 6, 11],
    ['bombing-damage', 0, 21, 6, 10], ['bombing-damage-breakdown', 6, 21, 6, 10],
    ['aces', 0, 31, 12, 10],
  ] },
  { id: 'navy', label: 'Navy', icon: 'anchor', panels: [
    ['navy-size', 0, 0, 6, 10], ['naval-losses', 6, 0, 6, 10],
    ['naval-battles', 0, 10, 12, 11],
    ['ships-sunk', 0, 21, 12, 10],
  ] },
  { id: 'occupation', label: 'Occupation', icon: 'flag', panels: [
    ['territory', 0, 0, 6, 10], ['territory-breakdown', 6, 0, 6, 10],
    ['occupied-states', 0, 10, 12, 11],
    ['occupation-trend', 0, 21, 12, 10],
  ] },
  { id: 'intelligence', label: 'Intelligence', icon: 'eye', panels: [
    ['agency-comparison', 0, 0, 6, 10], ['decryption', 6, 0, 6, 10],
    ['who-spies-on-whom', 0, 10, 7, 12], ['operatives-roster', 7, 10, 5, 12],
    ['espionage-log', 0, 22, 6, 10], ['raid-tally', 6, 22, 6, 10],
    ['raid-log', 0, 32, 12, 10],
  ] },
] as const satisfies readonly { id: string; label: string; icon: IconName; panels: readonly Panel[] }[];

/** A built-in page id, or `page-…` for one the user added. */
export type PageId = string;
export const pageDefinition = (id: PageId) => PAGES.find(page => page.id === id);
export const pageIcon = (id: PageId): IconName => pageDefinition(id)?.icon ?? 'layout';

/** The sidebar's pages in order, with the names the user gave them. */
export interface PageEntry { id: PageId; label: string }
const PAGE_LIST_KEY = 'hoi4-replay.analytics-pages.v1';
export const MAX_PAGE_LABEL = 60;
type PageStorage = Pick<Storage, 'getItem' | 'setItem'>;

export const defaultPages = (): PageEntry[] => PAGES.map(({ id, label }) => ({ id, label }));

export function parsePageList(input: unknown): PageEntry[] | null {
  if (!Array.isArray(input) || !input.length) return null;
  const pages: PageEntry[] = [];
  for (const entry of input) {
    const id = entry?.id, label = typeof entry?.label === 'string' ? entry.label.trim() : '';
    if (typeof id !== 'string' || !(pageDefinition(id) || /^page-[a-zA-Z0-9_-]{1,60}$/.test(id))
      || !label || label.length > MAX_PAGE_LABEL || pages.some(page => page.id === id)) return null;
    pages.push({ id, label });
  }
  return pages;
}

export function loadPages(storage?: PageStorage): PageEntry[] {
  try { const raw = (storage ?? localStorage).getItem(PAGE_LIST_KEY); return (raw && parsePageList(JSON.parse(raw))) || defaultPages(); }
  catch { return defaultPages(); }
}

export function savePages(pages: PageEntry[], storage?: PageStorage): string | null {
  try { (storage ?? localStorage).setItem(PAGE_LIST_KEY, JSON.stringify(pages)); return null; }
  catch { return 'Your pages apply for this session. This device could not save them.'; }
}

/** Moves the page at `from` so it lands before the page now at `to` (`to` may be the length). */
export function movePage(pages: PageEntry[], from: number, to: number): PageEntry[] {
  const next = [...pages];
  const [page] = next.splice(from, 1);
  next.splice(to > from ? to - 1 : to, 0, page);
  return next;
}
