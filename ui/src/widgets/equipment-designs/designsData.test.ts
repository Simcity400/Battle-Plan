import test from 'node:test';
import assert from 'node:assert/strict';
import { designsAt, designsKnown, designTypes, seenNote, seenText, validateDesigns } from './designsData.ts';

const design = (id: string, creator: [number, number, string][], obsolete: [number, number, string][], archetype = 'synthetic_rifle') => ({
  id, creator, obsolete, token: `${archetype}_1`, archetype, archetypeName: archetype === 'synthetic_rifle' ? 'Synthetic rifles' : 'Synthetic tanks',
  names: [{ firstHour: 0, name: `Design ${id}`, known: true, design: null }] });
const reply = () => validateDesigns({ branchId: '1', view: 'designs', lastHour: 200, licences: [], organisations: [], variants: {}, texts: [], icons: {}, warnings: [],
  designsFrom: 10, known: [[10, 200, '1']], holders: { '1': [{ hour: 0, tag: 'AAA' }] },
  designs: [
    design('a', [[10, 200, '1']], [[10, 99, '0'], [100, 200, '1']]),
    design('b', [[10, 200, '1']], [[10, 200, '1']]),
    design('c', [[50, 200, '1']], [[50, 200, '0']], 'synthetic_tank'),
  ],
  coverage: { firstHour: 0, lastHour: 200, complete: true }, pending: [] });

test('designs split current and obsolete, dated only as far as the reads prove', () => {
  const label = (hour: number) => `h${hour}`;
  const early = designsAt(reply(), 'AAA', 60, null);
  assert.deepEqual(early.map(row => [row.key, row.obsolete]), [['a', false], ['b', true], ['c', false]]);
  // Present at the first read: saved (and flagged obsolete) by then, maybe long before.
  assert.equal(seenText(early[1].firstSeen, label), 'By h10');
  assert.equal(seenText(early[1].seenObsolete, label), 'By h10');
  assert.equal(seenNote(early[1].firstSeen), 'Already so at the start of the recorded designs');
  // Read the hour before without it: first seen at its read.
  assert.equal(seenText(early[2].firstSeen, label), 'h50');
  assert.equal(seenNote(early[2].firstSeen), null);
  // The flag went from current to obsolete between consecutive reads: seen obsolete then.
  // No replacement design is recorded, so nothing is called superseded.
  const later = designsAt(reply(), 'AAA', 150, null);
  assert.equal(seenText(later[0].seenObsolete, label), 'h100');
  assert.equal(designsAt(reply(), 'AAA', 40, null).length, 2);
});

test('a design or flag first read after a gap is only bounded by that read', () => {
  const label = (hour: number) => `h${hour}`;
  // The designs were not recorded 60–79: d appears at 80, e turns obsolete at 80.
  const gapped = validateDesigns({ ...reply(), known: [[10, 59, '1'], [80, 200, '1']], designs: [
    design('d', [[80, 200, '1']], [[80, 200, '0']]),
    design('e', [[10, 59, '1'], [80, 200, '1']], [[10, 59, '0'], [80, 200, '1']]),
  ] });
  const [d, e] = designsAt(gapped, 'AAA', 100, null);
  assert.equal(seenText(d.firstSeen, label), 'By h80');
  assert.equal(seenNote(d.firstSeen), 'First shown after hours the designs were not recorded');
  assert.equal(seenText(e.seenObsolete, label), 'By h80');
  // Obsolete on both sides of the gap: it may have changed in between, so only the resumed read dates it.
  const resumed = validateDesigns({ ...gapped, designs: [design('g', [[10, 59, '1'], [80, 200, '1']], [[10, 59, '1'], [80, 200, '1']])] });
  const [g] = designsAt(resumed, 'AAA', 100, null);
  assert.equal(seenText(g.seenObsolete, label), 'By h80');
  assert.equal(seenNote(g.seenObsolete), 'First shown after hours the designs were not recorded');
  // Obsolete at its first read with no current read before it: a bound, not a date.
  const fresh = validateDesigns({ ...reply(), designs: [design('f', [[50, 200, '1']], [[50, 200, '1']])] });
  assert.equal(seenText(designsAt(fresh, 'AAA', 100, null)[0].seenObsolete, label), 'By h50');
});

test('included types filter by archetype and list what the designs use', () => {
  assert.deepEqual(designsAt(reply(), 'AAA', 60, new Set(['synthetic_tank'])).map(row => row.key), ['c']);
  assert.deepEqual(designTypes(reply()).map(type => type.value), ['synthetic_rifle', 'synthetic_tank']);
});

test('designs before the first complete read are unknown, not none', () => {
  assert.equal(designsKnown(reply(), 5), false);
  assert.equal(designsKnown(reply(), 60), true);
});
