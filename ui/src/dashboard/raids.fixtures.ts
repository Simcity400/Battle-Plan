import type { RaidEpisode, RaidLogReply, RaidPhaseRun } from '../generated/dashboard';

export const phaseRun = (firstHour: number, lastHour: number, phase: number, outcome = 0): RaidPhaseRun =>
  ({ firstHour, lastHour, phase, phaseName: `Phase ${phase}`, outcome, outcomeName: outcome ? `Outcome ${outcome}` : 'No result' });

/** A raid by `raider`: preparing from `firstHour`, launched at `launchedHour`, ended at `endedHour` (a success when
 * `succeeded`), read every hour of its life. */
export function syntheticEpisode(id: string, firstHour: number, raider: string | null, launchedHour: number | null, endedHour: number | null,
  succeeded: boolean, changes: Partial<RaidEpisode> = {}): RaidEpisode {
  const lastHour = endedHour === null ? firstHour + 100 : endedHour + 10;
  const runs: RaidPhaseRun[] = [];
  const preparingTo = (launchedHour ?? endedHour ?? lastHour + 1) - 1;
  if (preparingTo >= firstHour) runs.push(phaseRun(firstHour, preparingTo, 2));
  if (launchedHour !== null) runs.push(phaseRun(launchedHour, (endedHour ?? lastHour + 1) - 1, 4));
  if (endedHour !== null) runs.push(phaseRun(endedHour, lastHour, 5, succeeded ? 3 : 1));
  return {
    id, firstHour, lastHour, raider, victim: 'MOD B', victimMissing: null, raidType: 'mod/sky_raid', raidTypeName: 'Sky raid from a mod',
    icon: 'GFX_mod_raid_target', iconMissing: null, province: null, state: 7,
    phase: runs[runs.length - 1].phase, phaseName: null, outcome: runs[runs.length - 1].outcome, outcomeName: null, runs,
    preparation: preparingTo >= firstHour ? [[firstHour, preparingTo, 0, 1]] : [],
    launchedHour, launched: launchedHour !== null, endedHour, succeeded, ...changes,
  };
}

export const syntheticRaidLog = (episodes: RaidEpisode[], changes: Partial<RaidLogReply> = {}): RaidLogReply => ({
  available: true, unavailable: null, phasePreparing: 2, phaseEnded: 5, outcomeNone: 0,
  types: [{ token: 'mod/sky_raid', name: 'Sky raid from a mod', missing: null }, { token: 'mod/sea_raid', name: null, missing: 'Refused by the mod' }],
  artwork: [{ key: 'GFX_mod_raid_target', image: null, missing: 'Synthetic missing artwork' }],
  episodes, missing: [], warnings: [], ...changes,
});
