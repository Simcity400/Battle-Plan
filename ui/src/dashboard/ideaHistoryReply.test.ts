import test from 'node:test';
import assert from 'node:assert/strict';
import { validateIdeaHistoryReply, validateIdeaTooltipData } from './ideaHistoryReply.ts';

const reply = { branchId: '1', tag: 'GER', spans: [], ideas: [], artwork: [], holders: [], categories: [], unobserved: [], identityStarts: [], modifiers: [], texts: [], warnings: [] };

test('idea first views retain approximate lanes and mark held order and acquisition pending', () => {
  const held = { hour: 200, slot: 7, countryId: 9001, spans: [{ token: 'mod_spirit', firstHour: 200, lastHour: 200, slot: null, character: null }], preparing: ['order', 'acquired', 'acquisitionContext'] };
  const first = validateIdeaHistoryReply({ ...reply, held, approximate: [{ firstHour: 101, lastHour: 200 }] });
  assert.deepEqual(first.held, held);
  assert.deepEqual(first.approximate, [{ firstHour: 101, lastHour: 200 }]);
  assert.throws(() => validateIdeaHistoryReply({ ...reply, held: { ...held, hour: 201 } }), /malformed/);
  assert.throws(() => validateIdeaHistoryReply({ ...reply, approximate: [{ firstHour: 101, lastHour: 100 }] }), /malformed/);
});

test('an idea history reply without per-holder artwork is refused, not rendered', () => {
  assert.equal(validateIdeaHistoryReply(reply).tag, 'GER');
  const { holders: _, ...older } = reply;
  assert.throws(() => validateIdeaHistoryReply(older));
  assert.throws(() => validateIdeaHistoryReply({ ...reply, holders: [{ token: 'x', character: 'y', artwork: {} }] }));
});

test('shared tooltip data checks effect shapes and keeps mod text when its art cannot be drawn', () => {
  const sources = { ideas: [{ token: 'mod_idea', category: 'mod_category', slot: null, law: true, designer: false,
    effectsKnown: true, unknown: null, effects: [{ key: 'mod_effect', fixed5: '-123456', hidden: false }], targeted: [], equipment: [],
    research: [], traits: [], tooltips: [] }], modifiers: [], texts: [],
  artwork: [{ token: 'mod_idea', name: 'Mod idea', icon: 'file:///mod/icon', description: 'Mod description', baseDurationDays: null }] };
  const data = validateIdeaTooltipData(sources);
  assert.equal(data.artwork[0].name, 'Mod idea');
  assert.equal(data.artwork[0].icon, null);
  assert.throws(() => validateIdeaTooltipData({ ...sources, ideas: [{ ...sources.ideas[0], effects: [{ key: 'broken', fixed5: 'unknown', hidden: false }] }] }), /malformed/);
  assert.throws(() => validateIdeaTooltipData({ ...sources, artwork: [{ token: 'mod_idea' }] }), /malformed/);
  assert.throws(() => validateIdeaHistoryReply({ ...reply, ...sources, ideas: [{ ...sources.ideas[0], effects: [{ key: 'broken', fixed5: 'unknown', hidden: false }] }] }), /The idea history reply is malformed/);
  assert.throws(() => validateIdeaHistoryReply({ ...reply, ...sources, spans: [{ token: 'mod_idea', firstHour: 12, lastHour: 10 }] }), /The idea history reply is malformed/);
  const { identityStarts: _starts, ...missingStarts } = reply;
  assert.throws(() => validateIdeaHistoryReply(missingStarts), /The idea history reply is malformed/);
});
