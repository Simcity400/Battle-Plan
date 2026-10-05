import type { RaidRun, RaidsReply } from '../generated/dashboard';

export const syntheticRaidRun = (firstHour: number, lastHour = firstHour, changes: Partial<RaidRun> = {}): RaidRun => ({
  firstHour, lastHour, readHour: firstHour, raider: 'MOD A', victim: 'MOD B', type: 'mod/sky_raid', province: 1, state: 2,
  targetName: 'Modded target', icon: 'GFX_mod_raid_target', iconMissing: null,
  phase: 2, phaseName: 'Modded preparation', outcome: 0, outcomeName: 'No result', launched: false, succeeded: false, ...changes,
});
export const syntheticRaids = (changes: Partial<RaidsReply> = {}): RaidsReply => ({
  branchId: '1', available: true, unavailable: null, types: [{ token: 'mod/sky_raid', name: 'Sky raid from a mod', missing: null }],
  artwork: [{ key: 'GFX_mod_raid_target', image: null, missing: 'Synthetic missing artwork' }],
  episodes: [{ id: 'episode', start: { hour: 100 }, preparation: [[100, 7], [101, 8], [103, 8], [104, 8], [105, 8]], runs: [syntheticRaidRun(100, 101),
    syntheticRaidRun(103, 105, { phase: 5, phaseName: 'Modded ended', outcome: 3, outcomeName: 'Modded success', succeeded: true, launched: null })] }],
  series: [{ tag: 'MOD A', segments: [{ firstHour: 100, lastHour: 101 }, { firstHour: 103, lastHour: 105 }], membership: null,
    runs: [{ firstHour: 100, lastHour: 101, readHour: 100, launched: 0, succeeded: 0 },
      { firstHour: 103, lastHour: 105, readHour: 103, launched: 0, succeeded: 1 }] }],
  readHours: [100, 101, 103, 104, 105], unobserved: [{ firstHour: 102, lastHour: 102 }], warnings: [], ...changes,
});
