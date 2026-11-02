import type { RaidAt } from '../../dashboard/raids';
import type { RaidsReply } from '../../generated/dashboard';
import { raidIcon, raidPlace, raidPreparationRead, raidType, raidTypeChoices, raidTypeName } from '../../dashboard/raids.ts';
import { entryAt } from '../../dashboard/reads.ts';
import { runAt } from '../../charts/series.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { LogRow } from '../../charts/log/logData';
import { newestFirst } from '../../charts/log/logData.ts';
import { playbackRows, spanChanges } from '../../charts/playbackIntervals.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';

/** Chips cover only types this branch observed; All types still accepts every type. */
export function raidLogTypes(data: RaidsReply): { token: string; label: string; icon: string | null }[] {
  const used = new Map<string, RaidAt['latest']>();
  for (const episode of data.episodes) for (const run of episode.runs) if (run.type !== null && !used.has(run.type)) used.set(run.type, run);
  return raidTypeChoices(data.types.filter(type => used.has(type.token))).map(choice => ({ ...choice, icon: raidIcon(data, used.get(choice.token)!) }));
}

/** Phase, result, type and target changes date the row; hourly preparation,
 * witness changes, artwork failures and a repeated read after a gap do not. */
const sameRaidChange = (a: RaidAt['latest'], b: RaidAt['latest']) => a.phase === b.phase && a.outcome === b.outcome
    && a.raider === b.raider && a.victim === b.victim && a.type === b.type && a.province === b.province && a.state === b.state
    && a.targetName === b.targetName;
function changeRead(at: RaidAt['episode'], latest: RaidAt['latest']): RaidAt['latest'] {
  let index = at.runs.indexOf(latest);
  while (index > 0 && sameRaidChange(at.runs[index - 1], latest)) index--;
  return at.runs[index];
}

export function raidLogRows(data: RaidsReply, hour: number, types: readonly string[], matches: (tag: string | null, hour: number) => boolean,
  from: number | null): ReadonlyArray<LogRow<RaidAt>> {
  const selected = new Set(types);
  return newestFirst(data.episodes.flatMap(episode => {
    const latest = entryAt(episode.runs, hour);
    const change = latest && changeRead(episode, latest);
    if (!latest || (selected.size && !selected.has(latest.type ?? ''))
      || !matches(latest.raider, change!.firstHour)
      || (from !== null && change!.firstHour < from)) return [];
    return [{ id: episode.id, key: JSON.stringify([episode.id, change!.firstHour]), hour: change!.firstHour, by: true,
      item: { episode, latest, current: runAt(episode.runs, hour), hour } }];
  }));
}

export function raidPlayback(data: RaidsReply, types: readonly string[], matches: (tag: string | null, hour: number) => boolean, from: number | null) {
  const selected = new Set(types);
  function* changes() {
    for (const episode of data.episodes) {
      let changed = 0, previous = false, preparationIndex = 0;
      for (let index = 0; index < episode.runs.length; index++) {
        const run = episode.runs[index];
        if (index === 0 || !sameRaidChange(episode.runs[changed], run)) changed = index;
        const date = episode.runs[changed].firstHour;
        const shown = (!selected.size || selected.has(run.type ?? '')) && matches(run.raider, date) && (from === null || date >= from);
        if (shown) {
          yield* spanChanges([run]);
          if (run.phase === data.phasePreparing) {
            while (preparationIndex < episode.preparation.length && episode.preparation[preparationIndex][1] < run.firstHour) preparationIndex++;
            for (let at = preparationIndex; at < episode.preparation.length && episode.preparation[at][0] <= run.lastHour; at++) {
              yield Math.max(run.firstHour, episode.preparation[at][0]);
              yield Math.min(run.lastHour, episode.preparation[at][1]) + 1;
            }
          }
        } else if (previous) yield run.firstHour;
        previous = shown;
      }
    }
  }
  return playbackRows(changes(), hour => raidLogRows(data, hour, types, matches, from), (rows, hour) =>
    rows.some(row => row.item.current?.phase === data.phasePreparing
      && row.item.episode.preparation.some(([first, last, , step]) => first <= hour && hour <= last && step !== 0))
      ? { first: hour, end: hour + 1 } : { first: -Infinity, end: Infinity });
}

export function raidStatus(data: RaidsReply, at: RaidAt): string {
  const value = at.current ?? at.latest;
  const phase = value.phaseName ?? 'Unknown phase';
  // Outcome text is shown only once ended and excludes the no-outcome sentinel.
  // The product carries both the phase and outcome classifications.
  const outcome = value.phase === data.phaseEnded && value.outcome !== data.outcomeNone ? value.outcomeName ?? 'Unknown outcome' : null;
  const status = outcome === null ? phase : `${phase} · ${outcome}`;
  return at.current ? status : `Last known: ${status}`;
}
export function raidPreparation(data: RaidsReply, at: RaidAt): string | null {
  // Preparation hours are shown only during the preparation phase.
  if ((at.current ?? at.latest).phase !== data.phasePreparing) return null;
  const read = raidPreparationRead(at);
  return read ? `${usNumber.format(read.hours)} h prepared` : 'Preparation not known';
}
export function raidDescription(data: RaidsReply, at: RaidAt, raider: string, victim: string): string {
  const preparation = raidPreparation(data, at);
  return `${raider} targets ${victim} at ${raidPlace(at.latest)} with ${raidTypeName(raidType(data.types, at.latest))}. ${raidStatus(data, at)}.${preparation ? ` ${preparation}.` : ''}`;
}
export function raidTooltip(data: RaidsReply, at: RaidAt, raider: string, victim: string): TooltipContent {
  const type = raidType(data.types, at.latest);
  const preparing = (at.current ?? at.latest).phase === data.phasePreparing;
  const preparation = preparing ? raidPreparationRead(at) : null;
  return { title: raidTypeName(type), icon: raidIcon(data, at.latest) ?? undefined, rows: [
    { label: 'Raider', value: raider }, { label: 'Target country', value: victim }, { label: 'Target', value: raidPlace(at.latest) },
    { label: 'Phase / outcome', value: raidStatus(data, at) },
    ...(preparing ? [{ label: 'Hours prepared', value: preparation ? usNumber.format(preparation.hours) : 'Not known' }] : []),
    { label: 'Seen by', value: formatDateTime(at.episode.start.hour) },
  ] };
}
