import test from 'node:test';
import assert from 'node:assert/strict';
import { ideaInfo, localisedText, modifierFormat } from './ideaContent.ts';

test('shared idea content refuses malformed nested tooltip values without changing Ideas reply validation', () => {
  const idea = { token: 'mod_law', category: 'mod_category', slot: 'mod_slot', law: true, designer: false,
    effectsKnown: true, unknown: null, effects: [{ key: 'mod_bonus', fixed5: '123456', hidden: false }],
    targeted: [], equipment: [], research: [], traits: [], tooltips: [] };
  assert.equal(ideaInfo(idea), true);
  assert.equal(ideaInfo({ ...idea, effects: [{ key: 'mod_bonus', fixed5: 'unknown', hidden: false }] }), false);
  assert.equal(ideaInfo({ ...idea, traits: [{ token: 'mod_trait', known: true }] }), false);
  assert.equal(ideaInfo({ ...idea, equipment: [null] }), false);
  assert.equal(modifierFormat({ key: 'mod_bonus', flags: 0, precision: 2, feature: 0, index: null,
    label: null, postfix: null, postfixKey: null }), true);
  assert.equal(localisedText({ key: 'mod_text', text: null }), true);
});
