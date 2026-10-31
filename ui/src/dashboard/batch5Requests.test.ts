import test from 'node:test';
import assert from 'node:assert/strict';
import * as requests from './batch5Requests.ts';
import { unsignedId, U8_MAX } from './batch5Params.ts';

/** Each Rust request DTO, field for field: serde's camelCase name, its JSON form and whether
 * the DTO requires it. All of them deny unknown fields. */
type Kind = 'string' | 'i32' | 'u8' | 'u32' | 'usize' | 'bool' | 'strings' | readonly string[];
type Dto = Record<string, { kind: Kind; required?: true }>;
const DTOS: Record<string, Dto> = {
  'air-missions': { branchId: { kind: 'string', required: true }, tag: { kind: 'string', required: true }, firstHour: { kind: 'i32' },
    lastHour: { kind: 'i32' }, view: { kind: ['history', 'wings'] }, hour: { kind: 'i32' }, region: { kind: 'u32' } },
  'air-statistics': { branchId: { kind: 'string', required: true }, tag: { kind: 'string', required: true },
    channel: { kind: 'u8', required: true }, firstHour: { kind: 'i32' }, lastHour: { kind: 'i32' } },
  minefields: { branchId: { kind: 'string', required: true }, tags: { kind: 'strings' }, firstHour: { kind: 'i32' }, lastHour: { kind: 'i32' },
    view: { kind: ['history', 'reports'] }, region: { kind: 'u32' }, hour: { kind: 'i32' } },
  'naval-supremacy': { branchId: { kind: 'string', required: true }, tag: { kind: 'string', required: true }, firstHour: { kind: 'i32' },
    lastHour: { kind: 'i32' }, daily: { kind: 'bool' } },
  'regional-weather': { branchId: { kind: 'string', required: true }, firstHour: { kind: 'i32' }, lastHour: { kind: 'i32' },
    regions: { kind: 'strings' }, show: { kind: ['conditions', 'modifier', 'temperature'] }, modifier: { kind: 'string' }, daily: { kind: 'bool' },
    view: { kind: ['history', 'provinces'] }, region: { kind: 'string' }, hour: { kind: 'i32' } },
  'doctrine-mastery': { branchId: { kind: 'string', required: true }, tag: { kind: 'string', required: true }, firstHour: { kind: 'i32' },
    lastHour: { kind: 'i32' }, hour: { kind: 'i32' } },
  'occupation-reserve-manpower': { branchId: { kind: 'string', required: true }, view: { kind: ['race', 'holder'] }, tags: { kind: 'strings' },
    tag: { kind: 'string' }, firstHour: { kind: 'i32' }, lastHour: { kind: 'i32' }, top: { kind: 'usize' } },
  'intel-network-coverage': { branchId: { kind: 'string', required: true }, view: { kind: ['matrix', 'pair'] }, tags: { kind: 'strings' },
    shown: { kind: 'usize' }, touching: { kind: 'bool' }, owner: { kind: 'string' }, target: { kind: 'string' }, firstHour: { kind: 'i32' }, lastHour: { kind: 'i32' } },
  'naval-yards': { branchId: { kind: 'string', required: true }, tag: { kind: 'string', required: true }, firstHour: { kind: 'i32' },
    lastHour: { kind: 'i32' } },
};

const integer = (value: unknown, min: number, max: number) => typeof value === 'number' && Number.isInteger(value) && value >= min && value <= max;

function assertDeserializes(product: string, params: unknown) {
  // What the transport sends: the serialized JSON, parsed as the backend sees it.
  const sent = JSON.parse(JSON.stringify(params)) as Record<string, unknown>;
  const dto = DTOS[product];
  for (const [key, value] of Object.entries(sent)) {
    const field = dto[key];
    assert.ok(field, `${product} sends ${key}, which its DTO denies`);
    const kind = field.kind;
    const ok = Array.isArray(kind) ? kind.includes(value as string)
      : kind === 'string' ? typeof value === 'string'
      : kind === 'bool' ? typeof value === 'boolean'
      : kind === 'strings' ? Array.isArray(value) && value.every(item => typeof item === 'string')
      : kind === 'i32' ? integer(value, -2147483648, 2147483647)
      : kind === 'u8' ? integer(value, 0, 255)
      : kind === 'u32' ? integer(value, 0, 4294967295)
      : integer(value, 0, Number.MAX_SAFE_INTEGER);
    assert.ok(ok, `${product}.${key} = ${JSON.stringify(value)} is not a ${String(kind)}`);
  }
  for (const [key, field] of Object.entries(dto)) if (field.required) assert.ok(key in sent, `${product} omits required ${key}`);
}

test('every Batch 5 request deserializes into its Rust DTO', () => {
  const cases: [string, unknown][] = [
    ['air-missions', requests.airMissionsHistory('1', 'GER')],
    ['air-missions', requests.oneDay(requests.airMissionsHistory('1', 'GER'), 24)],
    ['air-missions', requests.airMissionsWings('1', 'GER', '73', 30)],
    ['air-statistics', requests.airStatistics('1', 'GER', '3')],
    ['air-statistics', requests.oneDay(requests.airStatistics('1', 'GER', '3')!, 24)],
    ['minefields', requests.minefieldsHistory('1')],
    ['minefields', requests.minefieldsHistory('1', ['GER', 'ENG@3'])],
    ['minefields', requests.minefieldReports('1', '73', 30)],
    ['naval-supremacy', requests.navalSupremacy('1', 'GER', 0, 95)],
    ['regional-weather', requests.weatherHistory('1', 0, 95, 'modifier', 'air')],
    ['regional-weather', requests.weatherHistory('1', 0, 95, 'conditions', 'air')],
    ['regional-weather', requests.weatherProvinces('1', '73', 30)],
    ['doctrine-mastery', requests.doctrineMastery('1', 'GER')],
    ['doctrine-mastery', requests.doctrineMasteryAt('1', 'GER', 30)],
    ['occupation-reserve-manpower', requests.occupationRace('1')],
    ['occupation-reserve-manpower', requests.occupationRace('1', ['GER'])],
    ['occupation-reserve-manpower', requests.occupationHolder('1', 'GER@7')],
    ['intel-network-coverage', requests.intelMatrix('1', 12.7)],
    ['intel-network-coverage', requests.intelMatrix('1', 12, ['GER'], true)],
    ['intel-network-coverage', requests.intelMatrix('1', 1, undefined, true)],
    ['intel-network-coverage', requests.intelPair('1', 'GER@7', 'ENG@3')],
    ['naval-yards', requests.navalYards('1', 'GER', 100)],
  ];
  for (const [product, params] of cases) {
    assert.ok(params, `${product} built no request`);
    assertDeserializes(product, params);
  }
});

test('region and channel ids go out as the integers the DTOs read, and invalid ids ask for nothing', () => {
  assert.equal(requests.airMissionsWings('1', 'GER', '73', 30)!.region, 73);
  assert.equal(requests.minefieldReports('1', '73', 30)!.region, 73);
  assert.equal(requests.airStatistics('1', 'GER', '3')!.channel, 3);
  for (const id of ['', '073', '-1', '1.5', 'x', '4294967296']) {
    assert.equal(requests.airMissionsWings('1', 'GER', id, 30), null, id);
    assert.equal(requests.minefieldReports('1', id, 30), null, id);
  }
  assert.equal(requests.airStatistics('1', 'GER', '256'), null);
  assert.equal(unsignedId('255', U8_MAX), 255);
  // The provinces view keeps the region as the canonical text its DTO reads.
  assert.equal(requests.weatherProvinces('1', '73', 30).region, '73');
});
