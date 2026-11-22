import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Case { id: string; mode: 'follow' | 'pin inside' | 'pin outside' | 'range'; scope: 'moment' | 'history' }
interface Read { main: string[]; drill: string[]; shown?: number | null; crash: string | null }
interface Step { hour: number; states: Record<string, Read> }

const OFFSET = 900, FIRST = 1000, LAST = 1009;
const OUTSIDE = 'This time is outside the recording.';
const RECORDED_ABSENCE = /does not exist at this date|no members|not recorded at this (date|hour|time|moment)|weren’t recorded at this time|wasn’t recorded at this time|unavailable at this time|Country unavailable|Selected country unavailable|not held at this date|not in service at this date/i;

function problems(item: Case, step: Step, replies: 'pending' | 'failing', label: string): string[] {
  const read = step.states[item.id];
  const at = step.hour + OFFSET;
  const outside = item.mode === 'pin outside' || item.mode === 'follow' && (at < FIRST || at > LAST);
  const found: string[] = [];
  if (read.crash) found.push(`crashed: ${read.crash}`);
  for (const [region, states] of [['main', read.main], ['drill', read.drill]] as const) for (const state of states) {
    if (outside && RECORDED_ABSENCE.test(state)) found.push(`${region} calls an hour outside the recording recorded absence: ${state}`);
    if (outside && replies === 'pending' && item.scope === 'moment' && state.startsWith('loading')) found.push(`${region} waits on a read for an hour outside the recording`);
    if (!outside && state.includes(OUTSIDE)) found.push(`${region} calls recorded hour ${at} outside the recording`);
    if (!outside && replies === 'failing' && state.startsWith('loading')) found.push(`${region} is still loading after every read failed`);
  }
  if (outside && typeof read.shown === 'number' && read.shown >= FIRST && read.shown <= LAST) found.push(`shows recorded hour ${read.shown} as current at ${at}`);
  return found.map(text => `${item.id} ${replies} ${label} (track hour ${at}): ${text}`);
}

test('every widget, through the real host on a track aligned 900 hours later, stepped and played across both ends in follow, pin and range: each region says outside the recording for an unrecorded hour and nothing else, never calls a recorded hour outside, never waits forever, and never shows a recorded hour as current past the end', () =>
  withNativePage('time-sweep', '/ui/src/dashboard/timeSweep.dom.fixture.tsx', page => page.stage('time sweep', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.timeSweep.${expression}`);
    assert.equal(await act<number>('preload()'), 114, 'every widget definition is in the sweep');
    const cases = await act<Case[]>('cases()');
    const found: string[] = [];
    for (const replies of ['pending', 'failing'] as const) for (const mode of ['follow', 'pin inside', 'pin outside', 'range'] as const) {
      const group = cases.filter(item => item.mode === mode);
      const steps: [string, Step][] = [];
      steps.push(['mounted at 95', { hour: 95, states: await act(`mount(${JSON.stringify(group.map(item => item.id))}, '${replies}', 95)`) }]);
      for (const hour of [100, 105, 109]) steps.push([`sought to ${hour}`, await act<Step>(`seek(${hour})`)]);
      steps.push(['stepped past the end', await act<Step>('step(1)')]);
      await act('seek(100)');
      steps.push(['played inside', await act<Step>('play(4, 1000)')]);
      steps.push(['played past the end', await act<Step>('play(1440, 400)')]);
      for (const [label, step] of steps) for (const item of group) found.push(...problems(item, step, replies, label));
    }
    assert.ok(cases.length > 400, `every widget in every supported time mode, with its breakdowns: ${cases.length}`);
    assert.deepEqual([...new Set(found)].slice(0, 60), []);
  }, 1_200_000)));
