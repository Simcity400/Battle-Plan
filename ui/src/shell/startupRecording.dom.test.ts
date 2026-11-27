import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('the launch recording opens before the viewer is ready and plays after the map refuses it', async () => {
  await withNativePage('startup-recording', '/ui/src/shell/startupRecording.dom.fixture.tsx', page => page.stage('launch recording', async () => {
    const run = <T>(call: string) => page.evaluate<T>(`window.startupRecording.${call}`);
    await run('mount()');
    await run('viewer(false)');
    assert.deepEqual(await run('launch()'), ['launch.battleplan']);
    assert.equal(await run('viewer(true, "Map could not draw the recording")'), false, 'the native window leaves the refusal card exposed');
    assert.deepEqual(await run('play()'), { playing: true, opened: ['launch.battleplan'], mapOpens: [], mapSeeks: [] });
    assert.deepEqual(await run('retryMap()'), { opened: ['launch.battleplan'], mapOpens: ['C:/recordings/launch.battleplan'], released: [], playing: true });
    assert.equal(await run('viewer(true, null, "C:/recordings/launch.battleplan", null)'), true,
      'acceptance lets the viewer draw its first frame');
  }));
});

test('choosing a recording before the launch snapshot arrives keeps the chosen recording', async () => {
  await withNativePage('startup-choice', '/ui/src/shell/startupRecording.dom.fixture.tsx', page => page.stage('explicit recording', async () => {
    const run = <T>(call: string) => page.evaluate<T>(`window.startupRecording.${call}`);
    await run('mount()');
    assert.deepEqual(await run('choose()'), ['chosen.battleplan']);
    assert.deepEqual(await run('launch()'), ['chosen.battleplan']);
    await run('deliverChoice()');
    await run('viewer(true, null, "launch.battleplan")');
    await run('viewer(true, null, "chosen.battleplan")');
    assert.deepEqual(await run('play()'), { playing: true, opened: ['chosen.battleplan'], mapOpens: ['chosen.battleplan'], mapSeeks: ['chosen.battleplan'] });
  }));
});

for (const launchFirst of [false, true]) {
  test(`a refused choice ${launchFirst ? 'after' : 'before'} the launch snapshot leaves the launch recording alone`, async () => {
    await withNativePage('startup-refused-choice', '/ui/src/shell/startupRecording.dom.fixture.tsx', page => page.stage('refused recording', async () => {
      const run = <T>(call: string) => page.evaluate<T>(`window.startupRecording.${call}`);
      await run('mount()');
      if (launchFirst) assert.deepEqual(await run('launch()'), ['launch.battleplan']);
      await run('choose()');
      assert.equal(await run('refuseChoice()'), 'This recording could not be opened');
      if (!launchFirst) assert.deepEqual(await run('launch()'), ['chosen.battleplan', 'launch.battleplan']);
      assert.deepEqual(await run('play()'), {
        playing: true,
        opened: launchFirst ? ['launch.battleplan', 'chosen.battleplan'] : ['chosen.battleplan', 'launch.battleplan'],
        mapOpens: [], mapSeeks: [],
      });
    }));
  });
}
