import { createServer } from 'vite';
import type { CountryGroupReply } from '../generated/dashboard';
import type { CountrySelection } from './model';

/** Synthetic fixtures for mounted tests of widgets that choose countries by group. */

/** The branch the fixtures cover; GER and ENG hold their tags through `PREPARED_THROUGH` unless `final`. */
export const BRANCH = { id: '1', firstHour: 0, lastHour: 200 };
export const PREPARED_THROUGH = 160;
/** An hour after `PREPARED_THROUGH`, and one before it. */
export const PENDING_HOUR = 170;
export const KNOWN_HOUR = 150;

/** The human players, counted at each hour or at the branch end. */
export const humansBy = (membership: 'hour' | 'end' | 'ever'): CountrySelection =>
  ({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false });

/** A groups reply: GER and ENG hold their tags from the first hour through `PREPARED_THROUGH`
 * (`final`: through the branch end), and GER is a human player through `PREPARED_THROUGH`. */
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

/** No human player, their membership not recorded at 140-150. */
export const gapGroups = (): CountryGroupReply =>
  ({ ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } });

/** Loads `entry` (a path under the repository root) through Vite's SSR loader with each
 * module whose path ends in a key of `replacements` replaced by its source. */
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
