import test from 'node:test';
import assert from 'node:assert/strict';
import { spriteFrames, spriteIcons, spriteId } from './spriteArtwork.ts';

const png = 'data:image/png;base64,iVBORw0KGgo=';

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

test('a sprite reply answering frames it was not asked for is malformed', () => {
  const asked = [{ key: 'GFX_a', frame: 0 }];
  assert.throws(() => spriteIcons({ items: [{ key: 'GFX_a', frame: 1, icon: png }], warnings: [] }, asked), /malformed/u);
  assert.throws(() => spriteIcons({ items: [{ key: 'GFX_a', frame: 0, icon: png }, { key: 'GFX_a', frame: 0, icon: png }], warnings: [] }, asked), /malformed/u);
  assert.throws(() => spriteIcons({ items: [] }, asked), /malformed/u);
});
