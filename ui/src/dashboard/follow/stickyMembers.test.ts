import assert from 'node:assert/strict';
import test from 'node:test';
import { stickyLimit, stickyMembers } from './stickyMembers.ts';

test('a top reordering, or a country leaving and coming back, names no new member, so nothing is followed again', () => {
  const first = stickyMembers(null, 'top 3', ['GER', 'SOV', 'USA'], stickyLimit(3));
  assert.deepEqual(first.tags, ['GER', 'SOV', 'USA']);
  assert.equal(stickyMembers(first, 'top 3', ['USA', 'GER', 'SOV'], stickyLimit(3)), first, 'reordered: the same set, the same object');
  const entered = stickyMembers(first, 'top 3', ['GER', 'ENG', 'USA'], stickyLimit(3));
  assert.deepEqual(entered.tags, ['ENG', 'GER', 'SOV', 'USA'], 'a new member is added; the one that left stays named');
  assert.equal(stickyMembers(entered, 'top 3', ['SOV', 'GER', 'USA'], stickyLimit(3)), entered, 'the one that left comes back: nothing new');
});

test('another query starts again from its own top, and a set past its limit starts again from the top now', () => {
  const held = stickyMembers(null, 'top 2 by army', ['GER', 'SOV'], 3);
  assert.deepEqual(stickyMembers(held, 'top 2 by navy', ['ENG', 'USA'], 3).tags, ['ENG', 'USA']);
  const full = stickyMembers(held, 'top 2 by army', ['GER', 'FRA'], 3);
  assert.deepEqual(full.tags, ['FRA', 'GER', 'SOV']);
  assert.deepEqual(stickyMembers(full, 'top 2 by army', ['ITA', 'JAP'], 3).tags, ['ITA', 'JAP'], 'over the limit: only the top now');
  assert.equal(stickyLimit(10), 64);
  assert.equal(stickyLimit(100), 400);
});
