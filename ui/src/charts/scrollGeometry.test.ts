import assert from 'node:assert/strict';
import test from 'node:test';
import { logicalScroll, MAX_DOM_SCROLL_EXTENT, physicalScroll, scrollGeometry, scrollItemTop, wheelScrollDelta } from './scrollGeometry.ts';


test('25 million rows retain the full scrollbar and ordinary row spacing within bounded DOM extent', () => {
  const geometry = scrollGeometry(25_000_000 * 38, 400);
  assert.equal(geometry.extent, MAX_DOM_SCROLL_EXTENT);
  assert.equal(geometry.compressed, true);
  for (const logical of [0, 38, 400_000_000, geometry.logicalRange]) {
    const physical = physicalScroll(geometry, logical);
    assert.ok(physical >= 0 && physical <= geometry.physicalRange);
    assert.ok(Math.abs(logicalScroll(geometry, physical) - logical) < 0.000001);
    assert.equal(scrollItemTop(geometry, logical + 38, logical) - scrollItemTop(geometry, logical, logical), 38);
  }
  assert.equal(physicalScroll(geometry, Infinity), geometry.physicalRange);
  assert.equal(logicalScroll(geometry, Infinity), geometry.logicalRange);
  assert.equal(scrollItemTop(geometry, 25_000_000 * 38, geometry.logicalRange), geometry.extent);
});

test('wheel scrolling keeps pixel, line and page distances independent of compression', () => {
  assert.equal(wheelScrollDelta(12.5, 0, 38, 400), 12.5);
  assert.equal(wheelScrollDelta(3, 1, 38, 400), 114);
  assert.equal(wheelScrollDelta(-1, 2, 38, 400), -400);
  const geometry = scrollGeometry(950_000_000, 400);
  const top = 400_000_000;
  const next = top + wheelScrollDelta(38, 0, 38, 400);
  assert.equal(next - top, 38);
  assert.ok(physicalScroll(geometry, next) - physicalScroll(geometry, top) < 1);
});
