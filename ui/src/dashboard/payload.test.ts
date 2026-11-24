import { test } from 'node:test';
import assert from 'node:assert/strict';
import { artworkName, artworkUrlValue } from './payload.ts';

test('product images are artwork URLs naming a content hash, nothing else', () => {
  const name = `${'0a'.repeat(16)}.png`;
  assert.equal(artworkName(name), true);
  for (const url of [`http://artwork.localhost/${name}`, `/api/artwork/${name}`]) assert.equal(artworkUrlValue(url), true);
  for (const value of [
    'data:image/png;base64,synthetic', 'guessed-art-name', name, `https://artwork.localhost/${name}`, `http://artwork.localhost/../${name}`,
    `/api/artwork/${name.toUpperCase()}`, `/api/artwork/${name}?x`, `/api/artwork/${name.slice(1)}`, `/api/artwork//${name}`, `artwork:${name}`, null,
  ]) assert.equal(artworkUrlValue(value), false, String(value));
  for (const value of [`../${name}`, `${name}/`, name.replace('.png', '.tmp'), '']) assert.equal(artworkName(value), false, value);
});
