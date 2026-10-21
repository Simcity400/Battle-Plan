import test from 'node:test';
import assert from 'node:assert/strict';
import { organisationsAt, policyText, rosterKnown, validateOrganisations } from './organisationsData.ts';
import { withWindowDiagnostics } from '../../dashboard/gameWindows.ts';

const mio = (id: string, size: string | null, funds: string, policy: number | null) => ({ id, holder: [[0, 50, '1']], size: size === null ? [] : [[0, 50, size]],
  funds: [[0, 50, funds]], points: [], unlocked: [[0, 50, '2']], policy: policy === null ? [] : [[0, 50, String(policy)]],
  policyName: policy === null ? [] : [[0, 50, '1']], policyUnresolved: id === '3' ? [[0, 50, '1']] : [], cooldown: [[0, 50, '100']],
  names: [{ firstHour: 0, token: 'synthetic_mio', name: id === '2' ? null : 'Synthetic Works', icon: 'GFX_synthetic' }] });
// References are decimal u64 strings, as every product spells them.
const reply = () => validateOrganisations({ branchId: '1', view: 'organisations', lastHour: 50, licences: [], designs: [], designsFrom: null, variants: {}, warnings: [],
  known: [[0, 50, '1']], holders: { '1': [{ hour: 0, tag: 'AAA' }] }, texts: ['synthetic_policy', 'Synthetic Policy'], icons: { GFX_synthetic: null },
  organisations: [mio('1', '2', '500000', 0), mio('2', '5', '-1', null), mio('3', null, '0', null)],
  coverage: { firstHour: 0, lastHour: 50, complete: true }, pending: [] });

test('organisations rank by size, keep unknown state unranked and name policies as stored', () => {
  const rows = organisationsAt(reply(), 'AAA', 40);
  assert.deepEqual(rows.map(row => row.key), ['2', '1', '3']);
  assert.equal(rows[0].name, null);
  assert.equal(rows[1].policy, 'Synthetic Policy');
  assert.equal(policyText(rows[0]), 'None · Cooldown 3 days');
  // A selected policy the recording cannot spell is not "None".
  assert.equal(policyText(rows[2]), 'Policy name unavailable · Cooldown 3 days');
  assert.equal(organisationsAt(reply(), 'BBB', 10).length, 0);
});

test('a running cooldown shows its remaining days', () => {
  const [, row] = organisationsAt(reply(), 'AAA', 30);
  assert.equal(policyText(row), 'Synthetic Policy · Cooldown 3 days');
});

test('an unread roster is unknown, not an empty one', () => {
  assert.equal(rosterKnown(reply(), 10), true);
  assert.equal(rosterKnown({ known: [] }, 10), false);
});

test('the policy window’s refusals join the catalogue warnings, neither overwriting the other', () => {
  const catalogue = ['Organisation names unavailable for 2 organisations'];
  const window = ['Policy button refused: unsupported effect', 'Artwork unavailable: GFX_synthetic_policy', catalogue[0]];
  assert.deepEqual(withWindowDiagnostics(catalogue, window), [catalogue[0], window[0], window[1]]);
  assert.equal(withWindowDiagnostics(catalogue, []), catalogue);
  assert.deepEqual(withWindowDiagnostics([], window.slice(0, 1)), window.slice(0, 1));
  // A line the window repeats (artwork failing again as a list scrolls) is reported once.
  assert.deepEqual(withWindowDiagnostics(catalogue, [window[1], window[1], window[0], window[1]]), [catalogue[0], window[1], window[0]]);
});
