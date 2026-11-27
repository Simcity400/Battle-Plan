import test from 'node:test';
import assert from 'node:assert/strict';
import { countryKey } from '../../dashboard/countryKey.ts';
import { readyArt, PENDING_ART } from '../../families/art/types.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import { barsLane, type FamilyBarsBinding } from '../../families/bars/laneView.ts';
import { layoutFamilyBars } from '../../families/bars/layout.ts';
import { collectVisibleBarsIcons } from '../../families/bars/painter.ts';
import type { FamilyTheme } from '../../families/contract.ts';
import { createLaneView } from '../../families/laneRuntime.ts';
import { assertQuietFrames } from '../../families/laneRuntime.fixture.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { knownValue } from '../../families/value.ts';
import type { FuelBranchesReply } from '../../generated/dashboard';
import {
  fuelBranchesDeclaration,
  fuelBranchesProjection,
  fuelBranchesSources,
  type FuelBranchesOptions,
} from './fuelBranchesProjection.ts';

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

function sampleReply(overrides: Partial<FuelBranchesReply> = {}): FuelBranchesReply {
  return {
    tag: 'GER',
    scale: 100000,
    consumerNames: ['Army', 'Air', 'Navy'],
    consumerIcons: [
      'http://artwork.localhost/army.png',
      'http://artwork.localhost/air.png',
      'http://artwork.localhost/navy.png',
    ],
    consumerIconReasons: [null, null, null],
    fuelIcon: 'http://artwork.localhost/fuel.png',
    fuelIconReason: null,
    readHour: 500,
    consumers: [
      { index: 0, requested: '19100000', received: '19100000', priority: 1 },
      { index: 1, requested: '16800000', received: '16800000', priority: 2 },
      { index: 2, requested: '4200000', received: '3100000', priority: 3 },
    ],
    missing: null,
    warnings: [],
    ...overrides,
  };
}

function project(data: FuelBranchesReply, digits = 0, hour = 500) {
  const model = fuelBranchesProjection.project(createBoundedReadView({
    hour: displayedHour(hour),
    panels: new Map([['main', data]]),
    art: { demandOwner: 'view', resolve: handle => (handle.kind === 'url' ? readyArt(handle.url) : PENDING_ART) },
  }), { surface: '#1e1e1e', digits });
  assertBarsModelValid(model, hour);
  return model;
}

test('fuel by branch projects branch insignia, need targets, dashed shortfall, and fuel icon readout', () => {
  const model = project(sampleReply());
  assert.equal(model.form, 'ordered');
  assert.equal(model.showAxis, false);
  const rows = model.rows as readonly BarsDataRow[];
  assert.deepEqual(rows.map(r => r.label), [knownValue('Army'), knownValue('Air'), knownValue('Navy')]);
  assert.equal(rows[0]!.readout?.primary, '191');
  assert.equal(rows[0]!.readout?.secondary, '/ 191 a day');
  assert.equal(rows[0]!.readout?.shortfall, undefined);
  assert.equal(rows[0]!.readout?.icon?.status, 'ready');
  assert.equal(rows[0]!.media?.art.status, 'ready');
  assert.equal(rows[2]!.target?.numeric, 42);
  assert.equal(rows[2]!.numeric, 31);
  assert.equal(rows[2]!.readout?.primary, '31');
  assert.equal(rows[2]!.readout?.secondary, '/ 42 a day');
  assert.equal(rows[2]!.readout?.shortfall, '−11');

  const layout = layoutFamilyBars(model, 720, 260, THEME);
  assert.ok(layout.showReadout && layout.readoutWidth >= 120);
  const urls = collectVisibleBarsIcons(model, layout, 0);
  assert.ok(urls.includes('http://artwork.localhost/army.png'));
  assert.ok(urls.includes('http://artwork.localhost/fuel.png'));
});

test('missing consumers, zero requests, and unavailable icons stay honest', () => {
  const missing = project(sampleReply({ consumers: [], readHour: null, missing: 'Fuel consumers are not recorded at this hour: queue overflow' }));
  assert.equal(missing.emptyKind, 'unavailable');
  assert.match(missing.empty ?? '', /not recorded/);

  const zero = project(sampleReply({
    consumerIcons: [null, null, null],
    consumerIconReasons: ['missing army icon', 'missing air icon', 'missing navy icon'],
    fuelIcon: null,
    fuelIconReason: 'missing fuel icon',
    consumers: [{ index: 0, requested: '0', received: '0', priority: 1 }],
  }));
  const row = zero.rows[0] as BarsDataRow;
  assert.equal(row.value?.value.kind, 'zero');
  assert.equal(row.readout?.tone, 'muted');
  assert.equal(row.readout?.primary, '0');
  assert.equal(row.media?.art.status, 'unavailable');
  assert.equal(row.readout?.icon?.status, 'unavailable');

  const blank = project(sampleReply({
    consumerNames: ['   ', 'Air', 'Navy'],
    consumers: [{ index: 0, requested: '100000', received: '100000', priority: 1 }],
  }));
  const blankRow = blank.rows[0] as BarsDataRow;
  assert.deepEqual(blankRow.label, { kind: 'notRecorded', reason: 'Branch name not recorded' });
  assert.equal(blankRow.numeric, 1);
});

test('source plan keys by recorded country slot and withholds while unresolved', () => {
  const mkCtx = (countrySubject: string | null) => ({
    track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: 100, lastHour: 900 },
    countrySubject,
    options: {},
  }) as unknown as FamilyPlanContext;

  assert.equal(fuelBranchesSources(mkCtx(null)).entries.length, 0);
  const resolved = fuelBranchesSources(mkCtx(countryKey(5)));
  assert.equal(resolved.entries.length, 1);
  const read = resolved.entries[0]!.read;
  assert.equal(read.shape, 'point');
  if (read.shape === 'point') {
    assert.deepEqual(read.params, { branchId: '1', country: 5 });
    assert.deepEqual(read.followQuery, { panel: 'fuel-branches', branchId: '1', country: 5 });
  }
  assert.equal(fuelBranchesDeclaration.countries.mode, 'single');
});

test('a quiet stretch of played hours makes no projection or request work', () => {
  const binding: FamilyBarsBinding<FuelBranchesOptions> = {
    family: 'bars',
    kind: 'fuel-branches',
    view: 'bars',
    options: { surface: '#1e1e1e', digits: 0 },
    sources: {},
    staticPanels: { main: sampleReply() },
    width: 640,
    height: 260,
    scrollTop: 0,
    theme: THEME,
    reducedMotion: true,
  };
  const quiet = assertQuietFrames(
    () => createLaneView(barsLane, () => {}),
    { label: 'quiet hours', binding, hour: 500 },
    24,
    { width: 640, height: 260, dpr: 1 },
  );
  assert.equal(quiet, Infinity);
});
