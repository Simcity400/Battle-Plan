import test from 'node:test';
import assert from 'node:assert/strict';
import { spriteFrames, spriteIcons, spriteId } from './spriteArtwork.ts';

const png = 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png';

test('sprite frames are asked once each, in a stable order, without missing keys', () => {
  assert.deepEqual(spriteFrames([{ key: 'GFX_b', frame: 0 }, null, { key: 'GFX_a', frame: 2 }, undefined, { key: '', frame: 0 },
    { key: 'GFX_b', frame: 0 }, { key: 'GFX_a', frame: -1 }]), [{ key: 'GFX_b', frame: 0 }, { key: 'GFX_a', frame: 2 }]);
});

test('sprite art maps each asked frame to its drawable image or null', () => {
  const asked = [{ key: 'GFX_a', frame: 0 }, { key: 'GFX_a', frame: 3 }, { key: 'GFX_c', frame: 0 }];
  const icons = spriteIcons({ items: [{ key: 'GFX_a', frame: 0, icon: png }, { key: 'GFX_a', frame: 3, icon: null },
    { key: 'GFX_c', frame: 0, icon: 'https://example.invalid/x.png' }], warnings: [] }, asked);
  assert.deepEqual(asked.map(sprite => icons.get(spriteId(sprite))), [png, null, null]);
});

test('a frame with the recording\'s own choice is asked and answered apart from the same frame without one', () => {
  assert.deepEqual(spriteFrames([{ key: 'GFX_b', frame: 1, recorded: 'GFX_a' }, { key: 'GFX_b', frame: 1 },
    { key: 'GFX_b', frame: 1, recorded: '' }]), [{ key: 'GFX_b', frame: 1, recorded: 'GFX_a' }, { key: 'GFX_b', frame: 1 }]);
  const asked = [{ key: 'GFX_b', frame: 1, recorded: 'GFX_a' }, { key: 'GFX_b', frame: 1 }];
  const icons = spriteIcons({ items: [{ key: 'GFX_b', frame: 1, recorded: 'GFX_a', icon: png },
    { key: 'GFX_b', frame: 1, icon: null }], warnings: [] }, asked);
  assert.deepEqual(asked.map(sprite => icons.get(spriteId(sprite))), [png, null]);
  assert.throws(() => spriteIcons({ items: [{ key: 'GFX_b', frame: 1, recorded: 'GFX_c', icon: png }], warnings: [] }, asked), /malformed/u);
});

test('frame ids keep their parts apart whatever the keys hold', () => {
  assert.notEqual(spriteId({ key: 'GFX_a:x', frame: 0, recorded: 'GFX_b' }), spriteId({ key: 'GFX_a', frame: 0, recorded: 'x:GFX_b' }));
  assert.notEqual(spriteId({ key: 'GFX_a', frame: 0 }), spriteId({ key: 'GFX_a', frame: 0, recorded: 'null' }));
});

test('a sprite reply answering frames it was not asked for is malformed', () => {
  const asked = [{ key: 'GFX_a', frame: 0 }];
  assert.throws(() => spriteIcons({ items: [{ key: 'GFX_a', frame: 1, icon: png }], warnings: [] }, asked), /malformed/u);
  assert.throws(() => spriteIcons({ items: [{ key: 'GFX_a', frame: 0, icon: png }, { key: 'GFX_a', frame: 0, icon: png }], warnings: [] }, asked), /malformed/u);
  assert.throws(() => spriteIcons({ items: [] }, asked), /malformed/u);
});
