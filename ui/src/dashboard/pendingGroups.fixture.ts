import { createServer } from 'vite';
import type { CountryGroupReply } from '../generated/dashboard';
import type { CountrySelection } from './model';


export const BRANCH = { id: '1', firstHour: 0, lastHour: 200 };
export const PREPARED_THROUGH = 160;
export const PENDING_HOUR = 170;
export const KNOWN_HOUR = 150;

export const humansBy = (membership: 'hour' | 'ever'): CountrySelection =>
  ({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false });

export function pendingGroups(options: { final?: boolean; humans?: CountryGroupReply['humans']; tags?: readonly string[] } = {}): CountryGroupReply {
  const through = options.final ? BRANCH.lastHour : PREPARED_THROUGH;
  const reply: CountryGroupReply = {
    branchId: BRANCH.id, warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [],
    unobserved: { humans: [], factions: [], wars: [] },
    humans: options.humans ?? [{ tag: 'GER', firstHour: BRANCH.firstHour, lastHour: PREPARED_THROUGH }],
    countryIdentities: (options.tags ?? ['GER', 'ENG']).map((tag, index) => ({ tag,
      identities: [{ firstHour: BRANCH.firstHour, lastHour: through, slot: index + 1, countryId: index + 101 }] })),
  };
  return reply;
}

export const gapGroups = (): CountryGroupReply =>
  ({ ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } });

export async function mountModule<T = Record<string, unknown>>(entry: string, replacements: Record<string, string>,
  mode?: string): Promise<{ module: T; close: () => Promise<void> }> {
  const vite = await createServer({ configFile: false, ...(mode ? { mode } : {}), optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, logLevel: 'silent',
    plugins: [{ name: 'mounted-replacements', enforce: 'pre', transform(_code, id) {
      return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1];
    } }] });
  try { return { module: await vite.ssrLoadModule(entry) as T, close: () => vite.close() }; }
  catch (error) { await vite.close(); throw error; }
}
