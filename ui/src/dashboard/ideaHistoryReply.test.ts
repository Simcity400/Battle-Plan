import test from 'node:test';
import assert from 'node:assert/strict';
import { holdingSince, holdingUntil, ideaHistoryProof, validateIdeaHistoryReply, validateIdeaTooltipData } from './ideaHistoryReply.ts';
import type { IdeaHistoryReply, IdeaHolding } from '../generated/dashboard';

const info = { token: 'mod_idea', category: 'mod_category', slot: null, law: true, designer: false,
  effectsKnown: true, unknown: null, nameMissing: null, effects: [{ key: 'mod_effect', fixed5: '-123456', hidden: false }], targeted: [], equipment: [],
  research: [], traits: [], tooltips: [] };
const artwork = { token: 'mod_idea', name: 'Mod idea', icon: 'file:///mod/icon', description: 'Mod description', baseDurationDays: null };
const holding: IdeaHolding = { token: 'mod_idea', firstHour: 10, lastHour: 20, start: 'seen', end: { kind: 'held' }, slot: null, character: null, look: 0 };
const reply = (): IdeaHistoryReply => ({ branchId: '1', tag: 'GER', asked: 'GER', hour: 20, nextChange: 30, unrecorded: null,
  holdings: [{ ...holding }], held: [0], looks: [{ info, artwork: { ...artwork } }], categories: [], unobserved: [{ firstHour: 5, lastHour: 6 }],
  modifiers: [], texts: [], warnings: [] });

test('an idea history reply answers one hour: nothing after it, every index in range, membership only of what is held', () => {
  const valid = validateIdeaHistoryReply(reply());
  assert.equal(valid.looks[0]!.artwork.icon, null, 'an icon the dashboard cannot draw is dropped, the name kept');
  const bad = (change: (value: IdeaHistoryReply) => unknown) => {
    const value = reply();
    change(value);
    assert.throws(() => validateIdeaHistoryReply(value), /The idea history reply is malformed/);
  };
  bad(value => { value.holdings[0] = { ...holding, firstHour: 21, lastHour: 20 }; });
  bad(value => { value.holdings[0] = { ...holding, lastHour: 21 }; });
  bad(value => { value.holdings[0] = { ...holding, look: 1 }; });
  bad(value => { value.holdings[0] = { ...holding, lastHour: 19 }; });
  bad(value => { value.holdings[0] = { ...holding, lastHour: 15, end: { kind: 'endedBy', hour: 16 } }; });
  bad(value => { value.holdings[0] = { ...holding, lastHour: 15, end: { kind: 'ended' } }; });
  bad(value => { value.nextChange = 20; });
  bad(value => { value.unobserved = [{ firstHour: 101, lastHour: 110 }, { firstHour: 105, lastHour: 120 }]; });
  bad(value => { value.looks[0] = { info: { ...info, effects: [{ key: 'broken', fixed5: 'unknown', hidden: false }] }, artwork }; });
});

test('a reply proves the hours from its own up to the one before its next change', () => {
  assert.deepEqual(ideaHistoryProof(reply(), [0, 20], 25), [20, 29]);
  assert.equal(ideaHistoryProof(reply(), [0, 20], 30), null);
  assert.deepEqual(ideaHistoryProof({ ...reply(), nextChange: null }, [0, 20], 20), [20, Number.MAX_SAFE_INTEGER]);
});

test('a holding’s dates say how much the recording saw: held before, first observed, ended, ended by, last seen', () => {
  assert.equal(holdingSince({ ...holding, start: 'heldBefore' }).label, 'Held before');
  assert.equal(holdingSince({ ...holding, start: 'firstObserved' }).label, 'First observed');
  assert.deepEqual(holdingUntil(holding), { label: 'Until', value: 'Held now' });
  assert.equal(holdingUntil({ ...holding, end: { kind: 'endedBy', hour: 30 } }).label, 'Ended by');
  assert.equal(holdingUntil({ ...holding, end: { kind: 'lastSeen' } }).label, 'Last seen');
});

test('shared tooltip data checks effect shapes and keeps mod text when its art cannot be drawn', () => {
  const sources = { ideas: [info], modifiers: [], texts: [], artwork: [{ ...artwork }] };
  const data = validateIdeaTooltipData(sources);
  assert.equal(data.artwork[0]!.name, 'Mod idea');
  assert.equal(data.artwork[0]!.icon, null);
  assert.throws(() => validateIdeaTooltipData({ ...sources, ideas: [{ ...info, effects: [{ key: 'broken', fixed5: 'unknown', hidden: false }] }] }), /malformed/);
  assert.throws(() => validateIdeaTooltipData({ ...sources, artwork: [{ token: 'mod_idea' }] }), /malformed/);
});
