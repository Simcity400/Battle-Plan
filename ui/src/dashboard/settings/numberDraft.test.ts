import test from 'node:test';
import assert from 'node:assert/strict';
import { leaveNumber, numberText, receiveNumber, typeNumber } from './numberDraft.ts';
import type { NumberDraft } from './numberDraft';
import type { OptionField } from '../options';

const axis = { key: 'yMin', section: 'axes', label: 'Value axis from', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null } as Extract<OptionField, { kind: 'number' }>;
const window = { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, default: 365 } as Extract<OptionField, { kind: 'number' }>;

/** Types `keys` one at a time, as the browser reports a number box's value, with the
 * parent saving each legal number and echoing it back as a stored value would be. */
function typeKeys(field: typeof axis, start: number | null, keys: readonly string[]) {
  let value = start;
  let draft: NumberDraft = { text: numberText(start) };
  const saves: (number | null)[] = [];
  for (const text of keys) {
    const step = typeNumber(field, draft, text, value);
    draft = step.draft;
    if (step.save !== undefined) {
      saves.push(step.save);
      // Stored settings go through JSON, which writes -0 as 0.
      value = JSON.parse(JSON.stringify(step.save));
      draft = receiveNumber(draft, value);
    }
  }
  return { draft, value, saves };
}

test('typing a negative number through -0 keeps its sign', () => {
  // A number box reports a lone minus as empty.
  const typed = typeKeys(axis, 10, ['', '-0', '-05']);
  assert.deepEqual(typed.saves, [-0, -5]);
  assert.equal(typed.value, -5);
  assert.equal(typed.draft.text, '-05');
});

test('a legal number saves at once; partial text waits until the box is left', () => {
  const typed = typeKeys(window, 365, ['3', '36', '365', '365']);
  assert.deepEqual(typed.saves, [36, 365], '3 is under the least, and an unchanged 365 saves nothing');
  // Leaving with text under the least puts the saved number back.
  assert.deepEqual(leaveNumber(window, { text: '3' }, 365), { draft: { text: '365' } });
  // Leaving empty means automatic where the field allows it, and the saved number elsewhere.
  assert.deepEqual(leaveNumber(axis, { text: '' }, 10), { draft: { text: '', sent: null }, save: null });
  assert.deepEqual(leaveNumber(window, { text: '' }, 365), { draft: { text: '365' } });
});

test('a value changed elsewhere replaces what the box shows', () => {
  assert.deepEqual(receiveNumber({ text: '12', sent: 12 }, 40), { text: '40' });
  assert.deepEqual(receiveNumber({ text: '12', sent: 12 }, null), { text: '' });
  assert.deepEqual(receiveNumber({ text: '1' }, 10), { text: '10' });
});
