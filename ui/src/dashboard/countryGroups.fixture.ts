import type { CountryGroupReply } from '../generated/dashboard';

export function countryGroupReply(overrides: Partial<CountryGroupReply> = {}): CountryGroupReply {
  return { branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [],
    factions: [], wars: [], humans: [], countryIdentities: [], unobserved: { humans: [], factions: [], wars: [] }, ...overrides };
}
