import { coverageAt, type CoveredBranch } from '../regionTime.ts';

export type StripDensity = 1 | 4 | 8 | 16;

export function stripLattice(branch: CoveredBranch, offset: number) {
  const first = branch.firstHour - offset, last = branch.lastHour - offset;
  const gaps: { first: number; last: number }[] = [];
  for (const gap of [...(branch.gaps ?? [])].sort((a, b) => a.firstHour - b.firstHour)) {
    const start = Math.max(first, gap.firstHour - offset), end = Math.min(last, gap.lastHour - offset);
    if (end < start) continue;
    const previous = gaps.at(-1);
    if (previous && start <= previous.last + 1) previous.last = Math.max(end, previous.last);
    else gaps.push({ first: start, last: end });
  }
  const stretches: typeof gaps = [];
  let start = first;
  for (const gap of gaps) {
    if (start < gap.first) stretches.push({ first: start, last: gap.first - 1 });
    start = gap.last + 1;
  }
  if (start <= last) stretches.push({ first: start, last });
  const bucketAt = (hour: number, width: number) => Math.floor((hour + offset + 1) / width);
  let bucket = 6;
  while (stretches.reduce((sum, span) => sum + bucketAt(span.last, bucket) - bucketAt(span.first - 1, bucket), 0) > 512) bucket *= 2;
  const hours = (density: StripDensity) => {
    const width = bucket * 16 / density;
    const hours: number[] = [];
    for (const span of stretches) {
      hours.push(span.first);
      for (let hour = (bucketAt(span.first, width) + 1) * width - 1 - offset; hour < span.last; hour += width) hours.push(hour);
      if (span.last > span.first) hours.push(span.last);
    }
    return hours;
  };
  return {
    stretches, gaps, last: stretches.at(-1)?.last ?? first - 1, bucket, hours,
    scrub(position: number, density: StripDensity) {
      const hour = Math.floor(position);
      if (hour < first || hour > last) return null;
      const stretch = stretches.find(span => hour >= span.first && hour <= span.last);
      if (!stretch) {
        const coverage = coverageAt({ branch }, hour + offset);
        return coverage.hour === null ? { hour, gap: coverage.outside } : null;
      }
      const width = bucket * 16 / density;
      return { hour: hour === stretch.last ? hour : Math.max(stretch.first, bucketAt(hour, width) * width - 1 - offset), gap: null };
    },
  };
}

export type StripLattice = ReturnType<typeof stripLattice>;
