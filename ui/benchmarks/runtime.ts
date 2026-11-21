import { performance } from 'node:perf_hooks';
import { WindowMap } from '../src/dashboard/windowMap.ts';
import { researchAt } from '../src/widgets/research/researchData.ts';
import { coverageAt, researchLanes, researchLanesAt } from '../src/widgets/research/researchLayout.ts';
import type { ResearchHistory } from '../src/generated/dashboard';
import { extentOf, sameStretch } from '../src/charts/series.ts';
const rows = Array.from({ length: 5000 }, (_, i) => ({ firstHour: i * 2, lastHour: i * 2, value: i % 37 }));
const countries = Array.from({ length: 1500 }, () => rows.slice());
function measure(label: string, run: () => void) {
  const timings: number[] = [];
  for (let i = 0; i < 12; i++) { const start = performance.now(); run(); timings.push(performance.now() - start); }
  timings.sort((a, b) => a - b);
  console.log(JSON.stringify({ label, countries: countries.length, rowsPerCountry: rows.length, medianMs: timings[6], maxMs: timings.at(-1) }));
}
let sink = 0;
measure('linear range fitting', () => { for (const runs of countries) {
  let low = Infinity, high = -Infinity;
  for (const run of runs) if (run.lastHour >= 8000 && run.firstHour <= 8020) { low = Math.min(low, run.value); high = Math.max(high, run.value); }
  sink += low + high;
} });
measure('indexed range fitting', () => { for (const runs of countries) { const range = extentOf(runs, 8000, 8020); sink += range?.[0] ?? 0; } });
console.log(JSON.stringify({ sink }));
measure('linear continuity playback lookup', () => { for (const runs of countries) sink += runs.findIndex(run => run.firstHour <= 9998 && 9998 <= run.lastHour); });
measure('binary continuity playback lookup', () => { for (const runs of countries) sink += Number(sameStretch(runs, 9998, 9998)); });

const entries: ResearchHistory['slots'][number]['entries'] = Array.from({ length: 5000 }, (_, i) => ({
  token: `technology-${i}`, technologyHandle: i, level: 0, continuityStartHour: 0,
  firstObservedHour: i * 2, startedAfterHour: i * 2 - 1, lastObservedHour: i * 2, endedByHour: i * 2 + 1, outcome: 'completed',
}));
const research: ResearchHistory = { coverage: [{ firstHour: 0, lastHour: 10000 }], slots: [{ index: 0,
  availability: [{ firstHour: 0, lastHour: 10000 }], entries, progress: [], states: [],
}] };
researchLanesAt(research, 9999);
measure('research full-history lanes: 120 playback ticks', () => { for (let hour = 9880; hour < 10000; hour++) sink += researchLanes(researchAt(research, hour), coverageAt(research.coverage, hour), hour)[0].items.length; });
measure('research indexed lanes: 120 playback ticks', () => { for (let hour = 9880; hour < 10000; hour++) sink += researchLanesAt(research, hour)[0].items.length; });
measure('copying 5000 window arrivals', () => { let windows = new Map<number, number>(); for (let i = 0; i < 5000; i++) { windows = new Map(windows); windows.set(i, i); } sink += windows.size; });
measure('shared 5000 window arrivals', () => { let windows = new WindowMap<number>(); for (let i = 0; i < 5000; i++) windows = windows.with(i, i); sink += windows.size; });
