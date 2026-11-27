import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { setToken } from '../../dashboard/options.ts';
import { PENDING_ART, readyArt } from '../../families/art/types.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { barsLane, type FamilyBarsBinding } from '../../families/bars/laneView.ts';
import { assertBarsModelValid, type BarsDataRow, type BarsModel } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { FamilyTheme } from '../../families/contract.ts';
import { IdentityIndex } from '../../families/identity/index.ts';
import { createSnapshotIdentityProvider } from '../../families/identity/provider.ts';
import { createLaneView } from '../../families/laneRuntime.ts';
import { assertQuietFrames } from '../../families/laneRuntime.fixture.ts';
import type { FamilyPlanContext, FamilyViewportRequest, PointSourceRead } from '../../families/plan.ts';
import type { FactionPoolKind, FactionPoolsReply } from '../../generated/dashboard';
import { FAMILY_VIEWS, resolveWidgetViewSwitch } from '../manifest.ts';
import { definition } from './definition.ts';
import { type FactionPooledStrengthOptions, validateFactionPools } from './factionPooledStrengthProjection.ts';

const ICON = 'http://artwork.localhost/0123456789abcdef0123456789abcdef.png';

const THEME: FamilyTheme = {
  surface: '#16181d',
  raised: '#1e2128',
  dark: true,
  text: '#eef1f6',
  muted: '#8c95a6',
  line: '#2e3440',
  lineSoft: '#222630',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e5534b',
  shortage: '#ff6b6f',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#8c95a6',
  hatch: '#6e7681',
  hatchEdge: '#484f58',
  font: 'Inter',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 6,
};

const baseReply = (over: Partial<FactionPoolsReply> = {}): FactionPoolsReply => ({
  pool: 'resources',
  status: 'ok',
  reason: null,
  faction: '9',
  decimals: 5,
  resources: [
    { ordinal: 2, token: 'steel', name: 'Steel', icon: ICON, amount: '106100000', nameReason: null, iconReason: null, missing: null },
    { ordinal: 1, token: 'oil', name: 'Oil', icon: null, amount: '9300000', nameReason: null, iconReason: 'The current installation has no icon for this resource', missing: null },
    { ordinal: 3, token: null, name: null, icon: null, amount: '0', nameReason: 'Name missing', iconReason: 'Name missing', missing: null },
  ],
  manpower: [],
  manpowerTotal: null,
  research: null,
  doctrines: [],
  warnings: [],
  ...over,
});

async function poolView(pool: FactionPoolKind) {
  const entry = FAMILY_VIEWS.find(item => item.kind === 'faction-pooled-strength' && item.view === pool);
  assert.equal(entry?.family, 'bars');
  assert.equal(entry?.viewOption, 'pool');
  const spec = await entry!.loadSpec();
  return {
    projection: spec.projection as BarsWidgetDeclaration<FactionPooledStrengthOptions>['projection'],
    declaration: spec.declaration as BarsWidgetDeclaration<FactionPooledStrengthOptions>,
  };
}

function makeIdentity(hour: number) {
  const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: '1', content: 1 });
  index.ingestStylesReply({
    branchId: '1',
    hour,
    styles: [
      { tag: 'GER', name: 'German Reich', color: [102, 102, 102], valid: [hour - 10, hour + 10], flagChoice: { keys: ['GER_fascism', null, null], reasons: [null, null, null] } },
      { tag: 'ITA', name: 'Italy', color: [40, 120, 60], valid: [hour - 10, hour + 10], flagChoice: { keys: ['ITA_fascism', null, null], reasons: [null, null, null] } },
    ],
    warnings: [],
  });
  index.setIdentities({
    branchId: '1',
    known: [{ firstHour: hour - 10, lastHour: hour + 10 }],
    unknown: [],
    countries: [
      { tag: 'GER', identities: [{ countryId: 7, slot: 3, firstHour: hour - 10, lastHour: hour + 10 }], unresolved: [] },
      { tag: 'ITA', identities: [{ countryId: 8, slot: 4, firstHour: hour - 10, lastHour: hour + 10 }], unresolved: [] },
    ],
    warnings: [],
  });
  return createSnapshotIdentityProvider(index);
}

async function project(pool: FactionPoolKind, data: FactionPoolsReply, hour = 500): Promise<BarsModel> {
  const { projection } = await poolView(pool);
  const model = projection.project(createBoundedReadView({
    hour: displayedHour(hour),
    panels: new Map([['main', data]]),
    identity: makeIdentity(hour),
    art: { demandOwner: 'view', resolve: handle => (handle.kind === 'url' ? readyArt(handle.url) : PENDING_ART) },
  }), { surface: '#1e1e1e', pool });
  assertBarsModelValid(model, hour);
  return model;
}

test('resources pool preserves product order, exact fixed-point amounts, art handles and missing refusals', async () => {
  const viewSwitch = resolveWidgetViewSwitch('faction-pooled-strength', definition);
  assert.equal(viewSwitch?.option, 'pool');
  assert.deepEqual(viewSwitch?.choices?.map(choice => choice.value), ['resources', 'manpower', 'research', 'doctrine']);

  const model = await project('resources', baseReply());
  assert.equal(model.form, 'ordered');
  assert.equal(model.showAxis, false);
  assert.equal(model.summaryText, '3 resources');
  const rows = model.rows as BarsDataRow[];
  assert.deepEqual(rows.map(row => row.key), ['resource:2', 'resource:1', 'resource:3']);
  assert.deepEqual(rows.map(row => row.readout?.primary), ['1,061', '93', '0']);
  assert.equal(rows[0]!.media?.kind, 'art');
  assert.equal(rows[0]!.media?.art.status === 'ready' ? rows[0]!.media.art.url : null, ICON);
  assert.equal(rows[1]!.media?.art.status, 'unavailable');
  assert.equal(rows[2]!.label?.kind, 'notRecorded');
  assert.equal(rows[2]!.readout?.tone, 'muted');
});

test('manpower, research and doctrine pools resolve recorded slots and honest status states', async () => {
  const manpower = await project('manpower', baseReply({
    pool: 'manpower',
    decimals: 0,
    resources: [],
    manpower: [
      { ordinal: 0, slot: 3, tag: null, countryReason: null, amount: '400', missing: null },
      { ordinal: 1, slot: null, tag: null, countryReason: 'Country 99 has no link', amount: '100', missing: null },
    ],
    manpowerTotal: '500',
  }));
  assert.equal(manpower.summaryText, '500 manpower');
  const mpRows = manpower.rows as BarsDataRow[];
  assert.deepEqual(mpRows[0]!.label, { kind: 'known', value: 'German Reich' });
  assert.equal(mpRows[0]!.readout?.primary, '400');
  assert.equal(mpRows[0]!.tooltip?.items?.find(item => item.label === 'Share of the pool')?.value, '80%');
  assert.equal(mpRows[1]!.label?.kind, 'notRecorded');

  const research = await project('research', baseReply({
    pool: 'research',
    resources: [],
    research: {
      scientistBase: '125000',
      scientistBaseMissing: null,
      members: [{ slot: 3, tag: null, countryReason: null }, { slot: 4, tag: null, countryReason: null }],
      membersMissing: null,
    },
  }));
  assert.equal(research.summaryText, '2 sharing');
  const resRows = research.rows as BarsDataRow[];
  assert.deepEqual(resRows[0]!.label, { kind: 'known', value: 'Scientist base' });
  assert.equal(resRows[0]!.readout?.primary, '1.25');
  assert.deepEqual(resRows[1]!.label, { kind: 'known', value: 'German Reich' });
  assert.deepEqual(resRows[2]!.label, { kind: 'known', value: 'Italy' });

  const doctrine = await project('doctrine', baseReply({
    pool: 'doctrine',
    decimals: 0,
    resources: [],
    doctrines: [
      { ordinal: 0, token: 'land_doctrine', name: 'Land Doctrine', reason: null },
      { ordinal: 1, token: 'blank_doctrine', name: '   ', reason: null },
    ],
  }));
  assert.equal(doctrine.summaryText, '2 doctrines');
  assert.deepEqual((doctrine.rows[0] as BarsDataRow).label, { kind: 'known', value: 'Land Doctrine' });
  assert.deepEqual((doctrine.rows[1] as BarsDataRow).label, { kind: 'notRecorded', reason: 'Doctrine name not recorded' });

  const blankResource = await project('resources', baseReply({
    resources: [{ ordinal: 0, token: 'steel', name: '   ', icon: null, amount: '500000', nameReason: null, iconReason: null, missing: null }],
  }));
  assert.deepEqual((blankResource.rows[0] as BarsDataRow).label, { kind: 'notRecorded', reason: 'Resource name not recorded' });
  assert.equal((blankResource.rows[0] as BarsDataRow).readout?.primary, '5');

  const unrecorded = await project('resources', baseReply({ status: 'poolNotRecorded', reason: 'Resources unread', resources: [] }));
  assert.equal(unrecorded.emptyKind, 'unavailable');
  assert.equal(unrecorded.empty, 'Resources unread');

  const locked = await project('manpower', baseReply({ pool: 'manpower', status: 'poolLocked', reason: 'Locked', resources: [] }));
  assert.equal(locked.emptyKind, 'empty');
});

test('source plan keys by recorded slot, honours explicit faction override, and withholds while slot is unresolved', async () => {
  const { declaration } = await poolView('resources');
  const mkCtx = (countrySubject: string | null, options: Record<string, unknown> = {}) => ({
    track: { source: { id: 's1', revision: 'r1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: 100, lastHour: 900 },
    widget: { countries: { kind: 'tags', tags: ['GER'] } },
    countrySubject,
    options,
  }) as unknown as FamilyPlanContext;

  const bySlot = declaration.sources(mkCtx(countryKey(3)), {} as FamilyViewportRequest);
  const slotRead = bySlot.entries[0]!.read as PointSourceRead;
  assert.deepEqual(slotRead.params, { branchId: '1', country: 3, pool: 'resources' });
  assert.deepEqual(slotRead.followQuery, { panel: 'faction-pools', branchId: '1', country: 3, pool: 'resources' });

  const pending = declaration.sources(mkCtx(null), {} as FamilyViewportRequest);
  assert.equal(pending.entries.length, 0, 'withholds request while slot is unresolved');

  const explicit = declaration.sources(mkCtx(null, { faction: setToken('9') }), {} as FamilyViewportRequest);
  assert.deepEqual((explicit.entries[0]!.read as PointSourceRead).params, { branchId: '1', faction: '9', pool: 'resources' });
});

test('a quiet stretch of played hours does not re-project or request work', async () => {
  await poolView('resources');
  const binding: FamilyBarsBinding<FactionPooledStrengthOptions> = {
    family: 'bars',
    kind: 'faction-pooled-strength',
    view: 'resources',
    options: { surface: '#1e1e1e', pool: 'resources' },
    sources: {},
    staticPanels: { main: baseReply() },
    width: 640,
    height: 300,
    scrollTop: 0,
    theme: THEME,
    reducedMotion: true,
  };
  const quiet = assertQuietFrames(
    () => createLaneView(barsLane, () => {}),
    { label: 'quiet hours', binding, hour: 500 },
    24,
    { width: 640, height: 300, dpr: 1 },
  );
  assert.equal(quiet, Infinity);
});

test('validator rejects malformed replies', () => {
  assert.equal(validateFactionPools(baseReply()).resources.length, 3);
  assert.throws(() => validateFactionPools({ ...baseReply(), pool: 'bogus' }), /malformed/);
  assert.throws(() => validateFactionPools({
    ...baseReply(),
    resources: [{ ordinal: 1, token: 'oil', name: 'Oil', icon: null, amount: null, nameReason: null, iconReason: null, missing: null }],
  }), /malformed/);
});
