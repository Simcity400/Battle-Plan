import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { cooldownDays, cooldownValues, fundsText, organisationNumbers, organisationRows, policyText, validateOrganisations } from './organisationsData.ts';
import type { IndustrialOrganisationsReply, OrganisationNow } from '../../generated/dashboard';

const organisation = (id: string, read: boolean, policy: Partial<OrganisationNow> = {}): OrganisationNow => ({ id, token: 'generic_artillery_organization', name: 'Artillery Manufacturer',
  nameUnknown: null, icon: 'sprite:a#1', iconUnknown: null, size: read ? 6 : null, traits: read ? 3 : null,
  policyToken: null, policyName: null, policyUnknown: null, cooldownHour: null, ...policy });

const HOUR = 24_005;
const reply: IndustrialOrganisationsReply = { branchId: '1', tag: 'GER', missing: null, detailsMissing: null, warnings: [], icons: [{ key: 'sprite:a#1', image: 'artwork:a.png' }], organisations: [
  organisation('1', true, { policyToken: 'mio_policy_general_assembly_line_optimisers', policyName: 'Assembly Line Optimisers', cooldownHour: 1_012 * 24 + 3 }),
  organisation('2', true),
  organisation('3', true, { policyToken: 'mio_policy_unnamed', policyUnknown: 'The content does not name the selected policy.' }),
  organisation('4', false),
] };

test('a policy is named, none is none only where the state was read, and an unnamed selected policy is still a policy', () => {
  const rows = organisationRows(validateOrganisations(reply));
  assert.deepEqual(rows.map(row => policyText(row, cooldownDays(row.cooldownHour, HOUR))), ['Assembly Line Optimisers · Cooldown 12 days', 'None', 'Policy name unavailable', '—']);
  assert.equal(rows[0].icon, 'artwork:a.png');
});

test('cooldown days are counted from the shown hour by whole days, as the game counts them, and none once the day is reached', () => {
  const rows = organisationRows(reply);
  assert.equal(cooldownDays(rows[0].cooldownHour, HOUR), 12);
  assert.equal(cooldownDays(rows[0].cooldownHour, 1_011 * 24 + 23), 1, 'the last hour of the day before');
  assert.equal(cooldownDays(rows[0].cooldownHour, 1_012 * 24), null, 'its own day: none left');
  assert.equal(cooldownValues(rows, HOUR).get('1'), 12);
  assert.equal(cooldownValues(rows, HOUR).get('2'), 0);
});

test('funds and points are read from their cells, and a cell without a value is missing, never zero', () => {
  const values = valuesOf(new Map([[liveKey('1', 'funds'), 158_543_576], [liveKey('1', 'points'), 4]]), 'IndustrialOrganisationDetails was not recorded at this hour');
  const read = organisationNumbers(values, '1');
  assert.equal(fundsText(read.funds), '1,585');
  assert.equal(read.points, 4);
  const unread = organisationNumbers(values, '4');
  assert.equal(fundsText(unread.funds), '—', 'unread funds are missing, never zero');
  assert.equal(unread.points, null);
});

test('a reply with a malformed organisation is refused', () => {
  assert.throws(() => validateOrganisations({ ...reply, organisations: [{ ...reply.organisations[0], cooldownHour: 'soon' as unknown as number }] }), /Invalid industrial organisations/);
});
