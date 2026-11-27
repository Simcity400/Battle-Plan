import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

let seen = () => {};
const held = { pattern: null as RegExp | null, requests: [] as (() => void)[], seen: new Promise<void>(resolve => { seen = resolve; }) };
const HOLD = { name: 'hold-modules', configureServer(server: { middlewares: { use(handle: (request: { url?: string }, response: unknown, next: () => void) => void): void } }) {
  server.middlewares.use((request, _response, next) => { if (held.pattern?.test(request.url ?? '')) { held.requests.push(next); seen(); } else next(); });
} };

type Mode = 'follow' | 'pin inside' | 'pin gap' | 'pin outside' | 'range';
interface Case { id: string; mode: Mode; scope: 'moment' | 'history' }
interface Read { main: string[]; waiting: string[]; drawn: boolean; drill: string[]; shown?: number | null; crash: string | null }
interface Step { hour: number; states: Record<string, Read> }
interface Asked { product: string; hour: number }

const OFFSET = 900, FIRST = 1000, LAST = 1009, GAP = [1005, 1006];
const OUTSIDE = 'This time is outside the recording.', UNRECORDED = 'Not recorded at this time.';
const RECORDED_ABSENCE = /does not exist at this date|no members|not recorded at this (date|hour|time|moment)|weren’t recorded at this time|wasn’t recorded at this time|unavailable at this time|Country unavailable|Selected country unavailable|not held at this date|not in service at this date/i;
const recorded = (hour: number) => hour >= FIRST && hour <= LAST && !(hour >= GAP[0] && hour <= GAP[1]);

function problems(item: Case, step: Step, replies: 'pending' | 'failing', label: string, playing: boolean): string[] {
  const read = step.states[item.id];
  const at = Math.floor(step.hour) + OFFSET;
  const pinned = item.mode === 'pin inside' ? FIRST + 3 : item.mode === 'pin gap' ? FIRST + 5 : item.mode === 'pin outside' ? FIRST + 15 : null;
  const hour = pinned ?? (item.mode === 'range' ? FIRST + 8 : at);
  const outside = !recorded(hour);
  const found: string[] = [];
  if (read.crash) found.push(`crashed: ${read.crash}`);
  if (!read.main.length && !read.drawn && !playing) found.push('shows neither a state nor a drawn view for its main region');
  for (const [region, states] of [['main', read.main], ['drill', read.drill]] as const) for (const state of states) {
    if (outside && RECORDED_ABSENCE.test(state.replace(UNRECORDED, ''))) found.push(`${region} calls an hour outside the recording recorded absence: ${state}`);
    if (outside && replies === 'pending' && item.scope === 'moment' && state.startsWith('loading')) found.push(`${region} waits on a read for an hour outside the recording`);
    if (!outside && (state.includes(OUTSIDE) || state.includes(UNRECORDED))) found.push(`${region} calls recorded hour ${hour} outside the recording`);
    if (!outside && replies === 'failing' && state.startsWith('loading') && !playing) found.push(`${region} is still loading after every read failed`);
  }
  if (item.mode === 'pin gap' && item.scope === 'moment' && !playing && !read.main.some(state => state.includes(UNRECORDED)) && !read.main.every(state => state.startsWith('empty: Choose')))
    found.push(`a pin inside the declared gap does not say the time was not recorded: ${JSON.stringify(read.main)}`);
  if (outside && typeof read.shown === 'number' && recorded(read.shown)) found.push(`shows recorded hour ${read.shown} as current at ${hour}`);
  return found.map(text => `${item.id} ${replies} ${label} (track hour ${at}): ${text}`);
}

const unrecordedAsks = (asked: Asked[], label: string) => asked.filter(ask => !recorded(ask.hour)).map(ask => `${label}: ${ask.product} asked at unrecorded hour ${ask.hour}`);

test('every widget, through the real host on a track aligned 900 hours later with a declared gap, stepped and played across both ends in follow, pin and range: each region always shows a state, says outside or not recorded for an unrecorded hour and nothing else, never asks an unrecorded hour, never waits forever, and is inspected while playing as well as paused', () =>
  withNativePage('time-sweep', '/ui/src/dashboard/timeSweep.dom.fixture.tsx', page => page.stage('time sweep', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.timeSweep.${expression}`);
    assert.equal(await act<number>('preload()'), 114, 'every widget definition is in the sweep');
    const cases = await act<Case[]>('cases()');
    const found: string[] = [];
    for (const replies of ['pending', 'failing'] as const) for (const mode of ['follow', 'pin inside', 'pin gap', 'pin outside', 'range'] as const) {
      const group = cases.filter(item => item.mode === mode);
      const steps: [string, Step, boolean][] = [];
      steps.push(['mounted at 95', { hour: 95, states: await act(`mount(${JSON.stringify(group.map(item => item.id))}, '${replies}', 95)`) }, false]);
      found.push(...unrecordedAsks(await act<Asked[]>('asked()'), `${mode} ${replies} mounted at 95`));
      for (const hour of [100, 104, 105, 107, 109]) {
        steps.push([`sought to ${hour}`, await act<Step>(`seek(${hour})`), false]);
        if (hour === 100 && replies === 'pending' && mode === 'follow')
          assert.deepEqual(steps.at(-1)![1].states['ruling-party|follow|main']?.waiting, ['Reading the party history…'], 'a widget held by a required read names that read');
        found.push(...unrecordedAsks(await act<Asked[]>('asked()'), `${mode} ${replies} sought to ${hour}`));
      }
      steps.push(['stepped past the end', await act<Step>('step(1)'), false]);
      for (const [label, from, speed, count] of [['inside', 100, 4, 10], ['past the end', 105, 8, 9]] as const) {
        await act(`seek(${from})`);
        const played = await act<{ playing: Step & { advancing: boolean }; paused: Step }>(`play(${speed}, ${count}, 100)`);
        assert.ok(played.playing.advancing, `${mode} ${replies}: the clock is still playing when ${label} is sampled, at ${played.playing.hour}`);
        steps.push([`playing ${label}`, played.playing, true], [`paused after playing ${label}`, played.paused, false]);
        found.push(...unrecordedAsks(await act<Asked[]>('asked()'), `${mode} ${replies} played ${label}`));
      }
      for (const [label, step, playing] of steps) for (const item of group) found.push(...problems(item, step, replies, label, playing));
    }
    assert.ok(cases.length > 500, `every widget in every supported time mode, with its breakdowns: ${cases.length}`);
    assert.deepEqual([...new Set(found)].slice(0, 60), []);
  }, 1_500_000)));

test('a lane worker still loading the code two widgets need keeps the pipeline unprepared and the widgets loading, and once it has loaded it the widgets with nothing to read show their empty state', () =>
  withNativePage('time-sweep-held', '/ui/src/dashboard/timeSweep.dom.fixture.tsx', page => page.stage('held modules', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.timeSweep.${expression}`);
    await act('preload()');
    held.pattern = /\/ui\/src\/widgets\/(faction-upgrades\/factionUpgradesProjection|occupation-reserve-manpower\/occupationReserveBreakdownProjection)\.ts/;
    const ids = ['faction-upgrades|follow|main', 'occupation-reserve-manpower|follow|breakdown'];
    await act(`mount(${JSON.stringify(ids)}, 'failing', 100, false)`);
    await held.seen;
    const before = await act<Record<string, Read>>('states()');
    assert.equal(await act<boolean>('prepared()'), false, 'the workers have not loaded the projections they were asked for');
    assert.deepEqual(ids.map(id => before[id]!.main.map(state => state.split(':')[0])), [['loading'], ['loading']]);
    held.pattern = null;
    for (const next of held.requests.splice(0)) next();
    const after = await act<Record<string, Read>>('whenPrepared()');
    assert.equal(await act<boolean>('prepared()'), true);
    assert.deepEqual(ids.map(id => after[id]!.main), [['empty: This country is in no faction at this date.'], ['empty: Choose a holder: click its bar in the race.']]);
  }, 300_000), [HOLD]));
