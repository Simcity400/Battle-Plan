import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { describeRun } from '../../charts/timeline/runState.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import type { HourSpan, WarCasualtyRun, WarObservation, WarParticipantRun, WarTimeline, WarContributionGroup } from '../../generated/dashboard';
import { labelObservationAt, sideLabel } from './warsData.ts';

type Country = CountryLabel;
type Day = (hour: number) => string;
const singularSide = (label: string) => label === 'Attackers' ? 'Attacker' : label === 'Defenders' ? 'Defender' : label;

function endLabel(run: { firstHour: number; lastHour: number }, by: boolean, until: number, branchLast: number,
  gaps: readonly HourSpan[], day: Day, participant = false): string {
  const ongoing = run.lastHour >= branchLast && until >= branchLast ? "Ongoing at the recording’s end"
    : participant ? 'Still fighting' : 'Ongoing';
  return describeRun([run], Math.min(until, branchLast), {
    since: () => ongoing, before: () => ongoing,
    ended: at => `${by ? 'by ' : ''}${day(at)}`,
    endedBy: at => `by ${day(at)}`,
    lastSeen: at => `Not recorded; last seen ${day(at)}`,
  }, gaps) ?? 'Not recorded';
}

export function casualtyRows(losses: WarCasualtyRun | null, sides: readonly [string, string], absent = 'Not recorded'): TooltipRow[] {
  if (!losses) return sides.map(label => ({ label: `${singularSide(label)} casualties`, value: absent, tone: 'muted' }));
  const rows: TooltipRow[] = [];
  for (const [index, side] of sides.entries()) {
    const known = losses[2 + index * 2] as string;
    const unknown = losses[3 + index * 2] as string;
    rows.push({ label: `${singularSide(side)} casualties`, value: formatAmount(known, 0, 0) });
    if (BigInt(unknown) !== 0n) rows.push({ label: `${singularSide(side)} unattributed casualties`, value: formatAmount(unknown, 0, 0) });
  }
  return rows;
}

export interface WarTooltipInput {
  war: WarTimeline;
  observation: WarObservation | null;
  losses: WarCasualtyRun | null;
  lossesPending?: boolean;
  lossesReason?: string | null;
  hour: number;
  until: number;
  branchLast: number;
  unobserved: readonly HourSpan[];
  branchGaps?: readonly HourSpan[];
  capitulationsUnobserved: readonly HourSpan[];
  country: Country;
  day: Day;
  participant?: { tag: string; run: WarParticipantRun | null; capitulations?: readonly number[] };
  contributions?: WarContributionGroup | null;
  contributionsPending?: boolean;
}

export function warTooltip(input: WarTooltipInput): TooltipContent {
  const { war, observation, losses, hour, until, branchLast, unobserved, capitulationsUnobserved, country, day, participant } = input;
  const coversHour = (gap: HourSpan) => gap.firstHour <= hour && hour <= gap.lastHour;
  const inGap = unobserved.some(coversHour) || input.branchGaps?.some(coversHour)
    || (!observation && war.firstHour <= hour && hour <= war.lastHour);
  const absent = inGap ? 'Not recorded' : hour < war.firstHour ? 'Not started' : hour > war.lastHour ? 'Ended' : null;
  const labelled = inGap ? null : hour > war.lastHour ? labelObservationAt(war, hour) : observation;
  const sides: [string, string] = [sideLabel('viewpoint', labelled, country), sideLabel('opponent', labelled, country)];
  const title = absent === 'Not started' ? absent : labelled?.name.known ? labelled.name.text : 'Not recorded';
  const rows: TooltipRow[] = [
    { label: 'Started', value: `${war.startedBy ? 'by ' : ''}${day(war.firstHour)}` },
    { label: 'Ended', value: endLabel(war, war.endedBy, until, branchLast, unobserved, day, false) },
  ];
  const missingCaps = capitulationsUnobserved.some(gap => gap.firstHour <= Math.min(war.lastHour, until) && gap.lastHour >= war.firstHour);
  if (participant) {
    const run = !inGap && participant.run && participant.run.firstHour <= hour && hour <= participant.run.lastHour ? participant.run : null;
    const outside = inGap ? 'Not recorded' : 'Not in this war at this hour';
    const side = !run ? outside : singularSide(sideLabel(run.side, labelled, country));
    rows.unshift({ label: 'Side', value: side });
    rows.push({ label: 'Joined', value: run ? `${run.joinedBy ? 'by ' : ''}${day(run.firstHour)}` : outside },
      { label: 'Left', value: run ? endLabel(run, run.leftBy, until, branchLast, unobserved, day, true) : outside });
    if (run && !absent) {
      const contribution = input.contributions?.participants.find(row => row.tag === participant.tag);
      if (contribution) {
        rows.push({ label: 'Participation', value: `${formatAmount(BigInt(contribution.participation) * 100n, 5, 5)}%` },
          { label: 'Participation score', value: formatAmount(contribution.score, 5, 5) });
        const components = [
          ['Equipment damage', contribution.equipmentDamage], ['Occupation', contribution.occupation],
          ['Air damage', contribution.airDamage], ['Strategic bombing', contribution.strategicBombing],
          ['Ship damage', contribution.shipDamage], ['Convoy damage', contribution.convoyDamage],
          ['Casualties contribution', contribution.casualties], ['Unknown casualties contribution', contribution.unknownCasualties],
          ['Lend-lease sent', contribution.lendLeaseSent], ['Lend-lease received', contribution.lendLeaseReceived],
        ];
        for (const [label, raw] of components) rows.push({ label, value: formatAmount(raw, 5, 5) });
      } else rows.push({ label: 'Participation', value: input.contributionsPending ? 'Loading…' : 'Not recorded' });
    }
    const events = participant.capitulations ?? war.capitulations.filter(cap => cap.tag === participant.tag).map(cap => cap.hour);
    const caps = run ? events.filter(at => at <= until && run.firstHour <= at && at <= run.lastHour) : [];
    for (const cap of caps) rows.push({ label: 'Capitulated', value: day(cap) });
    if (!caps.length && (!run || missingCaps)) rows.push({ label: 'Capitulation', value: !run ? outside : 'Not recorded during gaps' });
  } else {
    const count = !absent && observation ? new Set([...observation.viewpointSide, ...observation.opposingSide]).size : null;
    rows.push({ label: 'Participants', value: count == null ? absent ?? 'Not recorded' : String(count), tone: count == null ? 'muted' : undefined });
    rows.push(...casualtyRows(absent ? null : losses, sides, absent ?? (input.lossesPending ? 'Loading…' : undefined)));
    if (!absent && observation) for (const [index, tags] of [observation.viewpointSide, observation.opposingSide].entries()) {
      rows.push({ label: sides[index], divider: true });
      for (const tag of new Set(tags)) {
        const score = input.contributions?.participants.find(row => row.tag === tag);
        rows.push({ label: country(tag).name, countryFlag: country(tag), indent: true, short: true,
          ...(score ? { value: `${formatAmount(BigInt(score.participation) * 100n, 5, 5)}% · ${formatAmount(score.score, 5, 5)}` } : {}) });
      }
    }
  }
  const notes = [
    !absent ? input.contributions?.reason : null,
    losses && !participant && !absent && losses[0] !== losses[1] ? `Casualties as recorded ${day(losses[0])} – ${day(losses[1])}.` : null,
    !losses && !participant && !absent && !input.lossesPending ? input.lossesReason : null,
    missingCaps ? 'Some capitulations may be missing.' : null,
  ].filter(Boolean).join(' ');
  const participantName = participant ? country(participant.tag) : null;
  return { title: participantName ? participantName.name : title, countryFlag: participantName ?? undefined, subtitle: `${participant ? title : 'War overview'} · ${day(hour)}`,
    rows, note: notes };
}
