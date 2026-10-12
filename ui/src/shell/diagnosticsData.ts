import type { BranchMetadata } from '../generated/dashboard';

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

/** Each branch's name, numbered within its session as the map settings number them;
 * the session is named only when the recording has several. */
export function branchLabels(branches: readonly Pick<BranchMetadata, 'id' | 'sessionId'>[]): Map<string, string> {
  const sessions = [...new Set(branches.map(branch => branch.sessionId))];
  return new Map(branches.map(branch => {
    const number = branches.filter(item => item.sessionId === branch.sessionId).indexOf(branch) + 1;
    return [branch.id, sessions.length > 1 ? `Session ${sessions.indexOf(branch.sessionId) + 1} · Branch ${number}` : `Branch ${number}`];
  }));
}
