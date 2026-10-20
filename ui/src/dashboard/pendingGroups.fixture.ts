import assert from 'node:assert/strict';
import { createServer } from 'vite';
import type { CountryGroupReply } from '../generated/dashboard';
import type { CountrySelection } from './model';

/** Synthetic fixtures for mounted tests of widgets that choose countries by group
 * while the groups reply is still being prepared. */

/** The branch the fixtures cover; the groups reply has read it through `PREPARED_THROUGH`. */
export const BRANCH = { id: '1', firstHour: 0, lastHour: 200 };
export const PREPARED_THROUGH = 160;
/** An hour still being prepared, and one already read. */
export const PENDING_HOUR = 170;
export const KNOWN_HOUR = 150;

/** The human players, counted at each hour or at the branch end. */
export const humansBy = (membership: 'hour' | 'end' | 'ever'): CountrySelection =>
  ({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false });

/** A groups reply answered from its prepared prefix: GER and ENG are known from the
 * first hour through `PREPARED_THROUGH`, GER a human player throughout, and every later
 * hour is still being prepared. `final` gives the same history as a final reply, in
 * which the hours after `PREPARED_THROUGH` are read and have no human player. */
export function pendingGroups(options: { final?: boolean; humans?: CountryGroupReply['humans']; tags?: readonly string[] } = {}): CountryGroupReply {
  const through = options.final ? BRANCH.lastHour : PREPARED_THROUGH;
  const reply: CountryGroupReply = {
    branchId: BRANCH.id, warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [],
    unobserved: { humans: [], factions: [], wars: [] },
    humans: options.humans ?? [{ tag: 'GER', firstHour: BRANCH.firstHour, lastHour: PREPARED_THROUGH }],
    countryIdentities: (options.tags ?? ['GER', 'ENG']).map((tag, index) => ({ tag,
      identities: [{ firstHour: BRANCH.firstHour, lastHour: through, slot: index + 1, countryId: index + 101 }] })),
  };
  return options.final ? reply : { ...reply, coverage: { firstHour: BRANCH.firstHour, lastHour: PREPARED_THROUGH, complete: false },
    pending: [{ firstHour: PREPARED_THROUGH + 1, lastHour: BRANCH.lastHour }] } as CountryGroupReply;
}

/** No human player found through `PREPARED_THROUGH`, their membership not recorded at 140-150,
 * and every later hour still being prepared. */
export const gapGroups = (): CountryGroupReply =>
  ({ ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } });

/** What a widget with no member to ask for (`noMembersAt`) shows over `gapGroups` with humans
 * counted at each hour: the recorded gap at 145, no member at 100, still being prepared at 170. */
export function assertNoMemberStates(render: (hour: number) => string) {
  assert.match(render(145), /Group membership is unavailable at this time\./);
  assert.match(render(100), /This group has no members at this moment\./);
  const later = render(PENDING_HOUR);
  assert.match(later, /aria-label="This range is still being prepared\."/);
  assert.doesNotMatch(later, /unavailable|no members/);
}

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
