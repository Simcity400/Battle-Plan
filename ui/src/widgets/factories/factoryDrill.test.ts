import assert from 'node:assert/strict';
import test from 'node:test';
import { runAt } from '../../charts/series.ts';
import { bindCountryParts, countryPartsRemainder } from '../../charts/race/countryPartsDrillData.ts';
import { factoriesRequest, factoryModel, validateFactoriesReply } from './factoriesData.ts';
import { factoryDrillData } from './factoryDrill.ts';
import type { CountryFactoriesReply } from '../../generated/dashboard';

const reply: CountryFactoriesReply = { branchId: 'branch', warnings: [],
  fields: [{ group: 'civilian', part: 'total' }, { group: 'civilian', part: 'damaged' }, { group: 'civilian', part: 'consumerGoods' },
    { group: 'military', part: 'total' }],
  groups: [{ key: 'civilian', flag: 'general_production', buildings: [{ token: 'mod works', name: 'Recorded works' }] },
    { key: 'military', flag: 'military_production', buildings: [{ token: 'mod forge', name: 'Recorded forge' }] }],
  series: [{ tag: 'country,with,separators', membership: [{ firstHour: 100, lastHour: 100 }],
    identities: [{ firstHour: 100, lastHour: 104, slot: 3, countryId: 9 }],
    reads: [{ firstHour: 100, lastHour: 100, readHour: 100, hourly: true },
      { firstHour: 101, lastHour: 101, readHour: 100, hourly: false },
      { firstHour: 102, lastHour: 102, readHour: 102, hourly: true },
      { firstHour: 104, lastHour: 104, readHour: 104, hourly: true }],
    segments: [{ firstHour: 100, lastHour: 102 }, { firstHour: 104, lastHour: 104 }],
    runs: [{ firstHour: 100, lastHour: 102, startsSegment: true, values: [5, 3, 4, 10] },
      { firstHour: 104, lastHour: 104, startsSegment: true, values: [0, 0, 0, 0] }] }] };

test('factory drill uses recorded groups and count formula outside top membership, preserving signs and gaps', () => {
  const model = factoryModel(validateFactoriesReply(reply));
  const drill = factoryDrillData(model, ['civilian', 'military'], 'free');
  assert.deepEqual(drill.parts(102).map(part => part.label), ['Recorded works', 'Recorded forge']);
  assert.deepEqual(drill.at('country,with,separators', 102)?.values, [-2, 10]);
  assert.equal(drill.at('country,with,separators', 102)?.total, 8);
  assert.equal(countryPartsRemainder(drill.at('country,with,separators', 102)), 0n);
  const bands = drill.bands('country,with,separators', { firstHour: 100, lastHour: 104 });
  assert.deepEqual(bands.map(runs => runAt(runs, 102)?.value), [-2, 10, 0]);
  assert.equal(drill.at('country,with,separators', 103), null);
  assert.ok(bands.every(runs => runAt(runs, 103) == null && runAt(runs, 104)?.breakBefore));
  assert.equal(drill.readHour('country,with,separators', 102), 102);
  assert.equal(drill.readHour('country,with,separators', 103), null);
  assert.equal(drill.at('country,with,separators', 104)?.total, 0);
  assert.equal(factoryDrillData(model, ['civilian'], 'built').at('country,with,separators', 102)?.total, 5);
  assert.deepEqual(factoriesRequest('branch', { tags: ['country,with,separators'] }, 'free', ['civilian']),
    { branchId: 'branch', tags: ['country,with,separators'], top: null });
});

test('factory read text, appearance/Jump date and key dates distinguish carried values from equal fresh confirmations', () => {
  const drill = factoryDrillData(factoryModel(validateFactoriesReply(reply)), ['civilian'], 'built');
  assert.equal(drill.readHour('country,with,separators', 101), 100);
  assert.equal(drill.readHour('country,with,separators', 102), 102);
  assert.equal(drill.at('country,with,separators', 101)?.note, undefined);
  assert.equal(drill.at('country,with,separators', 102)?.note, undefined);
  assert.deepEqual(drill.keyHours, [100, 101, 102, 104]);
  const carried = [...drill.tooltip('country,with,separators', 101, 'Recorded holder', new Map())!][0];
  assert.equal(carried.note, drill.at('country,with,separators', 101)?.note);
  const missing = factoryDrillData(factoryModel({ ...reply, series: reply.series.map(series => ({ ...series, reads: undefined })) }), ['civilian'], 'built');
  assert.equal(missing.readHour('country,with,separators', 101), null);
  assert.equal(missing.at('country,with,separators', 101)!.note, undefined);
});

test('factory history remains bound to the peaceful origin holder after rename and original-tag reuse', () => {
  const native: CountryFactoriesReply = { ...reply, fields: [{ group: 'civilian', part: 'total' }], series: [
    { tag: 'OLD', membership: null, segments: [{ firstHour: 100, lastHour: 101 }, { firstHour: 104, lastHour: 105 }],
      identities: [{ firstHour: 100, lastHour: 101, slot: 3, countryId: 9 }, { firstHour: 104, lastHour: 105, slot: 8, countryId: 99 }],
      reads: [{ firstHour: 101, lastHour: 101, readHour: 101, hourly: true }, { firstHour: 104, lastHour: 105, readHour: 104, hourly: true }],
      runs: [{ firstHour: 101, lastHour: 101, values: [12], startsSegment: true },
        { firstHour: 104, lastHour: 105, values: [99], startsSegment: true }] },
    { tag: 'NEW', membership: null, segments: [{ firstHour: 102, lastHour: 104 }],
      identities: [{ firstHour: 102, lastHour: 104, slot: 3, countryId: 9 }],
      reads: [{ firstHour: 102, lastHour: 104, readHour: 102, hourly: true }],
      runs: [{ firstHour: 102, lastHour: 104, values: [20], startsSegment: true }] },
  ] };
  const drill = factoryDrillData(factoryModel(validateFactoriesReply(native)), ['civilian'], 'built');
  const binding = bindCountryParts(drill, 'OLD', 100); // holder exists before its first factory relation
  assert.equal(binding.data.at('OLD', 100), null);
  assert.equal(binding.tagAt(104), 'NEW');
  assert.equal(binding.data.at('OLD', 104)?.total, 20);
  assert.equal(binding.data.at('OLD', 105), null);
  assert.equal(runAt(binding.data.bands('OLD', { firstHour: 100, lastHour: 105 })[0], 104)?.value, 20);
  assert.equal(runAt(binding.data.bands('OLD', { firstHour: 100, lastHour: 105 })[0], 105), null);
  assert.equal(bindCountryParts(drill, 'OLD', 99).data.at('OLD', 104), null);
});
