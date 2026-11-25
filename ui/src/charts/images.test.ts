import assert from 'node:assert/strict';
import test from 'node:test';
import { canvasImageRead } from './images.ts';

test('one canvas decoder distinguishes a pending URL, a decoded image and settled failure', () => {
  const before = globalThis.Image;
  const created: { complete: boolean; naturalWidth: number; onload?: () => void; onerror?: () => void }[] = [];
  globalThis.Image = class {
    complete = false; naturalWidth = 0; src = ''; onload?: () => void; onerror?: () => void;
    constructor() { created.push(this); }
  } as unknown as typeof Image;
  try {
    let loaded = 0, failed = 0;
    const wake = () => { loaded++; };
    assert.deepEqual(canvasImageRead('synthetic-decoder-good', wake), { image: null, pending: true, failed: false });
    assert.equal(created.length, 1);
    created[0].complete = true; created[0].naturalWidth = 18; created[0].onload!();
    assert.equal(loaded, 1, 'the existing callback set wakes once');
    assert.deepEqual(canvasImageRead('synthetic-decoder-good', wake), { image: created[0], pending: false, failed: false });
    const errorWake = () => { failed++; };
    assert.equal(canvasImageRead('synthetic-decoder-bad', errorWake).pending, true);
    created[1].complete = true; created[1].onerror!();
    assert.equal(failed, 1);
    assert.deepEqual(canvasImageRead('synthetic-decoder-bad', errorWake), { image: null, pending: false, failed: true });
    assert.equal(created.length, 2);
  } finally { globalThis.Image = before; }
});

test('a working set of 1,500 flags drawn in one pass is decoded once, pending or settled', () => {
  const before = globalThis.Image;
  let created = 0;
  globalThis.Image = class {
    complete = false; naturalWidth = 0; src = ''; onload?: () => void; onerror?: () => void;
    constructor() { created++; }
  } as unknown as typeof Image;
  try {
    const urls = Array.from({ length: 1500 }, (_, index) => `synthetic-working-set-${index}`);
    for (let pass = 0; pass < 3; pass++) for (const url of urls) canvasImageRead(url, () => {});
    assert.equal(created, urls.length);
  } finally { globalThis.Image = before; }
});
