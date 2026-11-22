import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

interface Shown { widget: number; text: string; dim: boolean; mode: string; target: number }

test('real lanes and the page install show every frame of every widget from one installed picture, never ahead, through play, pause, rewind, speed and pin', async t => {
  await withNativePage('install', '/ui/src/dashboard/follow/install.dom.fixture.tsx', page => page.stage('install', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.installFixture.${expression}`);
    assert.equal(await act<boolean | string>('start(12)'), true, 'every widget installed a picture after the first seek');
    const played = await act<{ frames: number; faults: string[]; installed: number }>('play(1440, 1500)');
    t.diagnostic(`played ${played.frames} frames, ${played.installed} widget pictures`);
    assert.deepEqual(played.faults, [], 'DOM, entry, date and value agree in every displayed frame while playing at 1,440 h/s');
    assert.ok(played.installed > played.frames * 6, 'pictures were shown while playing');
    const paused = await act<{ faults: string[]; shown: Shown[] }>('pause()');
    assert.deepEqual(paused.faults, []);
    assert.ok(paused.shown.every(s => s.mode === 'picture'), 'pausing hides nothing');
    const rewound = await act<{ first: Shown[]; faults: string[]; settled: Shown[] }>('rewind(500)');
    assert.ok(rewound.first.every(s => s.mode !== 'picture' || Number(s.text.split('|')[1]) <= 500), 'a rewind never shows a later hour, not for a frame');
    assert.deepEqual(rewound.faults, []);
    assert.ok(rewound.settled.every(s => s.mode === 'picture'), (rewound as unknown as { debug: string }).debug);
    const sped = await act<{ blanks: number; faults: string[] }>('speedChange(168)');
    assert.deepEqual([sped.blanks, sped.faults], [0, []], 'a speed change blanks nothing');
    const pinned = await act<{ immediate: Shown; settled: { mode: string; h: number; through: number } }>('pin(0, 50)');
    assert.ok(pinned.immediate.mode === 'reading' || Number(pinned.immediate.text.split('|')[1]) <= 50, 'a pin before what is shown never leaves a later hour visible');
    assert.ok(pinned.settled.mode === 'reading' || pinned.settled.h <= 50 && pinned.settled.through <= 50);
  }));
});
