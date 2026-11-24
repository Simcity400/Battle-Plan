import type { IconName } from '../components/Icons';
import type { WidgetKind } from './model';

type Panel = readonly [WidgetKind, number, number, number, number, string?];
export interface Page { id: string; label: string; icon: IconName; panels: readonly Panel[] }

export const PAGES = [
  { id: 'overview', label: 'Overview', icon: 'grid', panels: [
    ['map', 0, 0, 8, 14], ['event-feed', 8, 0, 4, 14],
    ['scoreboard', 0, 14, 8, 10], ['game-score', 8, 14, 4, 10],
    ['territory', 0, 24, 4, 10], ['faction-strength', 4, 24, 4, 10], ['world-tension', 8, 24, 4, 10],
    ['wars', 0, 34, 12, 9],
    ['apm-graph', 0, 43, 12, 9],
    ['apm-table', 0, 52, 10, 10],
    ['campaign-clock', 0, 62, 8, 8],
  ] },
  { id: 'politics', label: 'Politics', icon: 'parliament', panels: [
    ['national-focus', 0, 0, 4, 19], ['ruling-party', 4, 0, 8, 11], ['stability', 4, 11, 8, 8],
    ['political-power', 0, 19, 6, 10], ['political-power-spending-breakdown', 6, 19, 6, 10],
    ['political-power-spending', 0, 29, 12, 10],
    ['command-power', 0, 39, 6, 10], ['command-power-spending', 6, 39, 6, 10],
    ['national-spirits', 0, 49, 4, 8], ['spirit-history', 4, 49, 8, 8],
    ['idea-timeline', 0, 57, 12, 9],
    ['balance-of-power', 0, 66, 8, 12], ['next-elections', 8, 66, 4, 12],
    ['event-choices', 0, 78, 12, 12],
    ['decisions', 0, 90, 7, 12], ['governments-in-exile', 7, 90, 5, 12],
    ['civil-wars', 0, 102, 12, 10],
  ] },
  { id: 'diplomacy', label: 'Diplomacy', icon: 'handshake', panels: [
    ['factions', 0, 0, 6, 10], ['subjects', 6, 0, 6, 10],
    ['wars', 0, 10, 12, 10],
    ['world-tension', 0, 20, 6, 10], ['autonomy', 6, 20, 6, 10],
    ['peace-deals', 0, 30, 6, 10], ['territory-transfers', 6, 30, 6, 10],
    ['war-goals', 0, 40, 12, 9],
    ['war-contribution', 0, 49, 12, 10],
    ['faction-goals', 0, 59, 12, 10],
    ['faction-upgrades', 0, 69, 8, 9], ['faction-pooled-strength', 8, 69, 4, 9],
    ['diplomatic-relations', 0, 78, 6, 10], ['volunteers', 6, 78, 6, 10],
    ['diplomatic-pacts', 0, 88, 12, 10],
  ] },
  { id: 'economy', label: 'Economy', icon: 'factory', panels: [
    ['factories', 0, 0, 6, 10], ['equipment-produced', 6, 0, 6, 10],
    ['production-lines', 0, 10, 12, 11],
    ['production-spending', 0, 21, 6, 10], ['production-spending', 6, 21, 6, 10, 'factories=dockyards'],
    ['equipment-stockpile', 0, 31, 6, 10], ['equipment-designs', 6, 31, 6, 10],
    ['resource-balance', 0, 41, 6, 11], ['trade-flows', 6, 41, 6, 11],
    ['fuel', 0, 52, 4, 10], ['construction-spending', 4, 52, 8, 10],
    ['construction-log', 0, 62, 12, 10],
    ['fuel-branches', 0, 72, 6, 8], ['lend-lease-balance', 6, 72, 6, 10],
    ['lend-lease-agreements', 0, 82, 6, 10], ['market-ledger', 6, 82, 6, 10],
    ['international-market', 0, 92, 12, 10],
    ['production-licences', 0, 102, 6, 10], ['industrial-organisations', 6, 102, 6, 10],
    ['convoy-health', 0, 112, 12, 8],
    ['dockyard-spending', 0, 120, 12, 10],
    ['military-factory-spending', 0, 130, 12, 10],
  ] },
  { id: 'research', label: 'Research', icon: 'flask', panels: [
    ['active-research', 0, 0, 7, 9], ['research', 0, 9, 7, 13],
    ['special-projects', 7, 0, 5, 10], ['nuclear-arsenal', 7, 10, 5, 8],
    ['doctrine-mastery', 0, 22, 12, 12],
  ] },
  { id: 'land', label: 'Army', icon: 'helmet', panels: [
    ['army-size', 0, 0, 6, 10], ['casualties', 6, 0, 6, 10],
    ['land-battles', 0, 10, 12, 11],
    ['casualties-between', 0, 21, 6, 10], ['supply-health', 6, 21, 6, 10],
    ['generals', 0, 31, 7, 12], ['general-career', 7, 31, 5, 12],
    ['casualties-breakdown', 0, 43, 6, 10], ['equipment-losses', 6, 43, 6, 10],
    ['division-log', 0, 53, 8, 10], ['experience', 8, 53, 4, 10],
    ['army-losses-by-month', 0, 63, 6, 8], ['training-queue', 6, 63, 6, 8],
    ['division-equipment', 0, 71, 7, 11], ['division-template', 7, 71, 5, 11],
    ['most-decorated-divisions', 0, 82, 7, 10], ['division-service-record', 7, 82, 5, 10],
  ] },
  { id: 'air', label: 'Air', icon: 'plane', panels: [
    ['air-force-size', 0, 0, 6, 10], ['air-kills-losses', 6, 0, 6, 10],
    ['air-superiority-region', 0, 10, 6, 11], ['air-superiority-timeline', 6, 10, 6, 11],
    ['bombing-damage', 0, 21, 6, 10], ['bombing-damage-breakdown', 6, 21, 6, 10],
    ['aces', 0, 31, 12, 10],
    ['air-missions', 0, 41, 6, 10], ['air-statistics', 6, 41, 6, 10],
    ['weather-by-region', 0, 51, 12, 9],
  ] },
  { id: 'navy', label: 'Navy', icon: 'anchor', panels: [
    ['navy-size', 0, 0, 6, 10], ['naval-losses', 6, 0, 6, 10],
    ['naval-battles', 0, 10, 12, 11],
    ['ships-sunk', 0, 21, 12, 10],
    ['naval-supremacy', 0, 31, 12, 9],
    ['minefields', 0, 40, 6, 9], ['dockyards-and-repairs', 6, 40, 6, 9],
    ['refits', 0, 49, 6, 9],
  ] },
  { id: 'occupation', label: 'Occupation', icon: 'flag', panels: [
    ['territory', 0, 0, 6, 10], ['territory-breakdown', 6, 0, 6, 10],
    ['occupied-states', 0, 10, 12, 11],
    ['occupation-trend', 0, 21, 12, 10],
    ['occupation-reserve-manpower', 0, 31, 12, 10],
  ] },
  { id: 'intelligence', label: 'Intelligence', icon: 'eye', panels: [
    ['agency-comparison', 0, 0, 6, 10], ['decryption', 6, 0, 6, 10],
    ['who-spies-on-whom', 0, 10, 7, 12], ['operatives-roster', 7, 10, 5, 12],
    ['espionage-log', 0, 22, 6, 10], ['raid-tally', 6, 22, 6, 10],
    ['raid-log', 0, 32, 12, 10],
    ['intel-network-coverage', 0, 42, 6, 10],
  ] },
] as const satisfies readonly Page[];

export type PageId = string;
export const pageDefinition = (id: PageId): Page | undefined => PAGES.find(page => page.id === id);
export const pageIcon = (id: PageId): IconName => pageDefinition(id)?.icon ?? 'layout';

export interface PageEntry { id: PageId; label: string }
const PAGE_LIST_KEY = 'battleplan.analytics-pages.v1';
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
  catch { return 'Couldn’t save your pages on this device. They still apply for this session.'; }
}

export function movePage(pages: PageEntry[], from: number, to: number): PageEntry[] {
  const next = [...pages];
  const [page] = next.splice(from, 1);
  next.splice(to > from ? to - 1 : to, 0, page);
  return next;
}
