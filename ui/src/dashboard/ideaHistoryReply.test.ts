import test from 'node:test';
import assert from 'node:assert/strict';
import { validateIdeaHistoryReply } from './ideaHistoryReply.ts';

const reply = { branchId: '1', tag: 'GER', spans: [], ideas: [], artwork: [], holders: [], categories: [], unobserved: [], identityStarts: [], modifiers: [], texts: [], warnings: [] };

test('an idea history reply without per-holder artwork is refused, not rendered', () => {
  assert.equal(validateIdeaHistoryReply(reply).tag, 'GER');
  const { holders: _, ...older } = reply;
  assert.throws(() => validateIdeaHistoryReply(older));
  assert.throws(() => validateIdeaHistoryReply({ ...reply, holders: [{ token: 'x', character: 'y', artwork: {} }] }));
});
