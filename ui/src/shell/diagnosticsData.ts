import type { BranchMetadata } from '../generated/dashboard';
import { formatDate, formatDateTime, gameDate } from '../dashboard/calendar.ts';

/** A branch's time as one bar: where each reported gap sits, as percentages of its span. */
export function branchCoverage(branch: Pick<BranchMetadata, 'firstHour' | 'lastHour' | 'gaps'>) {
  const span = Math.max(1, branch.lastHour - branch.firstHour + 1);
  const gaps = branch.gaps.map(gap => ({ ...gap,
    left: (Math.max(gap.firstHour, branch.firstHour) - branch.firstHour) / span * 100,
    width: (Math.min(gap.lastHour, branch.lastHour) - Math.max(gap.firstHour, branch.firstHour) + 1) / span * 100 }));
  return { gaps, missingHours: branch.gaps.reduce((sum, gap) => sum + gap.lastHour - gap.firstHour + 1, 0) };
}

/** How much time a recording is missing, in the largest whole unit that reads naturally. */
export function missingTime(hours: number): string {
  if (hours < 48) return `${hours} ${hours === 1 ? 'hour' : 'hours'}`;
  const days = Math.round(hours / 24);
  return `${days} days`;
}

/** Each history's name: the main one, then each other by the day it was left behind, or where it starts when it
 * left none; histories sharing a name are numbered. */
export function historyLabels(branches: readonly Pick<BranchMetadata, 'id' | 'main' | 'fork' | 'firstHour'>[]): Map<string, string> {
  const day = (hour: number) => { const date = gameDate(hour); return date ? formatDate(date) : formatDateTime(hour); };
  const names = branches.map(branch => branch.main ? 'Main history'
    : branch.fork ? `Left behind ${day(branch.fork.hour)}` : `Separate, from ${day(branch.firstHour)}`);
  return new Map(branches.map((branch, index) => {
    const same = names.filter(name => name === names[index]).length;
    const number = names.slice(0, index).filter(name => name === names[index]).length + 1;
    return [branch.id, same > 1 ? `${names[index]} (${number})` : names[index]];
  }));
}
