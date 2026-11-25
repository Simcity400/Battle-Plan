import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Mounted { id: string; kind: string; track: 'early' | 'late'; follows: boolean }
interface Frame { hour: number; widgets: Record<string, { state: string; renders: number }> }
const OUTSIDE = 'outside: This time is outside the recording.';

test('every registered widget kind through the production host, half on a track ending at 100 and half on one starting at 101, playing at 1,440 h/s: frames between boundary crossings render nothing, and on the frame that crosses every following widget says outside for the track it leaves and clears it for the track it enters', async () => {
  await withNativePage('outside-recording', '/ui/src/dashboard/outsideRecording.dom.fixture.tsx', page => page.stage('outside recording', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.outsideFixture.${expression}`);
    const { mounted } = await act<{ mounted: Mounted[] } & Frame>('start(95)');
    assert.equal(mounted.length, 114, 'one of each registered widget kind is mounted');
    const [within, crossed, after] = await act<Frame[]>('play(1440, 3, 2.5)');
    const rendered = (frame: Frame) => mounted.filter(item => frame.widgets[item.id].renders > 0).map(item => `${item.kind}: ${frame.widgets[item.id].renders}`);
    assert.ok(within.hour > 95 && within.hour <= 100 && crossed.hour > 101 && after.hour > crossed.hour, `one frame crosses both ends: ${within.hour}, ${crossed.hour}, ${after.hour}`);
    assert.deepEqual(rendered(within), [], `a frame inside every widget’s state renders nothing at ${within.hour}`);
    assert.deepEqual(rendered(after), [], `a frame after the crossing renders nothing at ${after.hour}`);
    const following = mounted.filter(item => item.follows);
    assert.ok(following.some(item => item.track === 'early') && following.some(item => item.track === 'late'), `following widgets on both tracks: ${following.map(item => item.kind)}`);
    const wrong = following.flatMap(item => {
      const before = within.widgets[item.id].state, at = crossed.widgets[item.id].state;
      const leaves = item.track === 'early';
      return before.includes(OUTSIDE) === leaves || at.includes(OUTSIDE) !== leaves ? [`${item.kind} on ${item.track}: ${before} → ${at}`] : [];
    });
    assert.deepEqual(wrong, [], 'the crossing frame says outside for the track left and clears it for the track entered');
  }, 240_000));
});

const onPage = (run: (act: <T>(expression: string) => Promise<T>) => Promise<void>) => withNativePage('outside-widgets', '/ui/src/dashboard/outsideWidgets.dom.fixture.tsx',
  page => page.stage('outside widgets', () => run(expression => page.evaluate(`window.outsideWidgets.${expression}`)), 120_000));
const outside = OUTSIDE.split(': ')[1];

test('Equipment losses mounted at 99, before a country recorded 100–109, is outside the recording, and binds the country once the clock enters the track', () => onPage(async act => {
  const before = await act<string>("mount('losses', 99)");
  assert.ok(before.includes(outside) && !before.includes('Country unavailable'), `mounted at 99: ${before}`);
  const inside = await act<{ text: string; followed: unknown[] }>("seek('losses', 105)");
  assert.ok(!inside.text.includes('Country unavailable') && !inside.text.includes(outside), `at 105 the country recorded since 100 is bound: ${inside.text}`);
  assert.deepEqual(inside.followed, [{ panel: 'equipment-losses', branchId: '1', country: 1, landSource: 'destruction' }], 'its losses are followed for that country, and nothing was followed at 99');
}));
