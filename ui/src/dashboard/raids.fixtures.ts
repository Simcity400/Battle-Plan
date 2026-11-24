import type { RaidEpisode, RaidLogReply, RaidPhaseRun, RaidVictimAt, RaidVictimSource } from '../generated/dashboard';

export const phaseRun = (firstHour: number, lastHour: number, phase: number, outcome = 0, firstExact = true, afterHour: number | null = firstHour - 1, raider: string | null = null): RaidPhaseRun =>
  ({ firstHour, lastHour, firstExact, afterHour, raider, phase, phaseName: `Phase ${phase}`, outcome, outcomeName: outcome ? `Outcome ${outcome}` : 'No result' });

export function syntheticEpisode(id: string, firstHour: number, raider: string | null, launchedHour: number | null, endedHour: number | null,
  succeeded: boolean, changes: Partial<RaidEpisode> = {}): RaidEpisode {
  const lastHour = endedHour === null ? firstHour + 100 : endedHour + 10;
  const runs: RaidPhaseRun[] = [];
  const preparingTo = (launchedHour ?? endedHour ?? lastHour + 1) - 1;
  if (preparingTo >= firstHour) runs.push(phaseRun(firstHour, preparingTo, 2, 0, true, firstHour - 1, raider));
  if (launchedHour !== null) runs.push(phaseRun(launchedHour, (endedHour ?? lastHour + 1) - 1, 4, 0, true, launchedHour - 1, raider));
  if (endedHour !== null) runs.push(phaseRun(endedHour, lastHour, 5, succeeded ? 3 : 1, true, endedHour - 1, raider));
  const victim = 'victim' in changes ? changes.victim! : 'MOD B';
  const victimMissing = 'victimMissing' in changes ? changes.victimMissing! : (victim === null ? 'The raid\'s target names no country' : null);
  const victimSource: RaidVictimSource | null = 'victimSource' in changes
    ? changes.victimSource!
    : (victim === null ? null : (endedHour !== null ? 'controllerAtFirstEndedRead' : 'controllerAtObservedRead'));
  const victims: RaidVictimAt[] = 'victims' in changes
    ? changes.victims!
    : endedHour !== null && victim !== null && victimSource === 'controllerAtFirstEndedRead' && endedHour > firstHour
      ? [{ hour: firstHour, tag: victim, missing: null, source: 'controllerAtObservedRead' }, { hour: endedHour, tag: victim, missing: null, source: 'controllerAtFirstEndedRead' }]
      : [{ hour: firstHour, tag: victim, missing: victimMissing, source: victimSource }];
  return {
    id, firstHour, firstHourExact: true, lastHour, raider,
    launchedRaider: launchedHour !== null ? raider : null,
    endedRaider: endedHour !== null ? raider : null,
    victim, victimMissing, victimSource, victims,
    raidType: 'mod/sky_raid', raidTypeName: 'Sky raid from a mod',
    icon: 'GFX_mod_raid_target', iconMissing: null, province: null, state: 7,
    phase: runs[runs.length - 1].phase, phaseName: null, outcome: runs[runs.length - 1].outcome, outcomeName: null, outcomeMissing: null, runs,
    preparation: preparingTo >= firstHour ? [[firstHour, preparingTo, 0, 1]] : [],
    launchedHour, launchedExact: launchedHour !== null, launched: launchedHour !== null,
    endedHour, endedExact: endedHour !== null,
    endedSource: endedHour !== null ? 'observedEndedRead' : null,
    disappearedHour: null, disappearedExact: false,
    succeeded, ...changes,
  };
}

export const syntheticRaidLog = (episodes: RaidEpisode[], changes: Partial<RaidLogReply> = {}): RaidLogReply => ({
  available: true, unavailable: null, phasePreparing: 2, phaseEnded: 5,
  outcomeNone: 0, outcomeFailure: 1, outcomeLimitedSuccess: 2, outcomeSuccess: 3, outcomeCriticalSuccess: 4, outcomeCanceled: 5,
  types: [{ token: 'mod/sky_raid', name: 'Sky raid from a mod', missing: null }, { token: 'mod/sea_raid', name: null, missing: 'Refused by the mod' }],
  artwork: [{ key: 'GFX_mod_raid_target', image: null, missing: 'Synthetic missing artwork' }],
  episodes, missing: [], warnings: [], ...changes,
});
