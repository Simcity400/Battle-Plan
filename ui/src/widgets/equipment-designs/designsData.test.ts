import assert from 'node:assert/strict';
import test from 'node:test';
import { designTypes, designsAt, designsSince, seenText, validateDesigns } from './designsData.ts';
import type { DesignHistory, EquipmentDesignsReply } from '../../generated/dashboard';

const name = (id: string, token: string, typeName: string | null) => ({ id, hour: 1300, known: true, token, name: `Design ${id}`, typeName, nameUnknown: null, icon: null, iconUnknown: null });
const design = (id: string, lives: [number, number][], obsolete: [number, number][], token = 'infantry_equipment'): DesignHistory => ({ id, name: name(id, token, token === 'infantry_equipment' ? 'Infantry Equipment' : 'Tank'),
  lives: lives.map(([firstHour, lastHour]) => ({ firstHour, lastHour })), obsolete: obsolete.map(([firstHour, lastHour]) => ({ firstHour, lastHour })) });

const reply: EquipmentDesignsReply = { branchId: '1', tag: 'GER', warnings: [], icons: [], known: [{ firstHour: 1000, lastHour: 1099 }, { firstHour: 1150, lastHour: 1300 }],
  designs: [design('500', [[1000, 1300]], [[1200, 1300]]), design('502', [[1050, 1300]], []), design('503', [[1160, 1300]], [], 'tank')] };

test('a design is shown while it is the country\'s, with when it and its obsolete flag were first read and how much earlier they may be', () => {
  const rows = designsAt(validateDesigns(reply), 1250, null);
  assert.deepEqual(rows.map(row => [row.key, row.obsolete]), [['500', true], ['502', false], ['503', false]]);
  const label = (hour: number) => `h${hour}`;
  assert.deepEqual(rows.map(row => seenText(row.firstSeen, label)), ['By h1000', 'h1050', 'h1160']);
  assert.equal(seenText(rows[0].seenObsolete, label), 'h1200');
  assert.deepEqual(designsAt(reply, 1100, null).map(row => row.key), ['500', '502'], 'a design not yet saved is not shown');
  assert.deepEqual(designsAt(reply, 1250, new Set(['tank'])).map(row => row.key), ['503']);
  assert.deepEqual(designTypes(reply), [{ value: 'infantry_equipment', label: 'Infantry Equipment' }, { value: 'tank', label: 'Tank' }]);
});

test('the rows change only at a design\'s own hours, so following playback renders only there', () => {
  assert.equal(designsSince(reply, 1180), 1160);
  assert.equal(designsSince(reply, 1210), 1200);
  assert.equal(designsSince(reply, 1159), 1050);
});
