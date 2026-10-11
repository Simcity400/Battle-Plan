import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveOptions, sectionFields, setToken } from '../../dashboard/options.ts';
import { definition, normaliseAirOptions, OPTIONS } from './definition.ts';

test('one air View migrates saved charts and loss causes while retaining filters and presentation', () => {
  for (const view of ['race', 'lines']) for (const show of ['main', 'breakdown']) {
    const options = { show, view, regions: setToken('mod region'), causes: setToken('mod cause'), ratio: false, dateOverlay: true };
    const saved = { configVersion: 1, options } as unknown as Parameters<typeof definition.migrate>[1];
    const migrated = definition.migrate(1, saved);
    assert.equal(migrated.configVersion, 2);
    assert.deepEqual(migrated.options, { view: show === 'breakdown' ? 'breakdown' : view,
      regions: options.regions, causes: options.causes, ratio: false, dateOverlay: true });
    const resolved = resolveOptions(OPTIONS, migrated.options);
    assert.equal(resolved.view, show === 'breakdown' ? 'breakdown' : view);
    assert.equal(resolved.regions, options.regions);
    assert.equal(resolved.causes, options.causes);
    assert.deepEqual(normaliseAirOptions(migrated.options), migrated.options, 'normalisation is idempotent');
  }
  assert.deepEqual(normaliseAirOptions(), { view: 'race' });
  assert.deepEqual(normaliseAirOptions({ view: 'breakdown' }), { view: 'breakdown' });
  assert.equal(OPTIONS.some(field => field.key === 'show'), false);
  for (const view of ['race', 'lines', 'breakdown']) {
    const options = resolveOptions(OPTIONS, { view, ratio: false, dateOverlay: true, endLabels: false, playback: 'window', windowDays: 28, yMax: 500 });
    const fields = sectionFields(OPTIONS, 'options', options).map(field => field.key);
    assert.equal(fields.filter(key => key === 'view').length, 1);
    assert.equal(fields.includes('ratio'), view === 'race');
    assert.equal(fields.includes('dateOverlay'), view === 'race');
    assert.equal(fields.includes('endLabels'), view === 'lines');
    assert.equal(sectionFields(OPTIONS, 'interaction', options).some(field => field.key === 'drill'), view !== 'breakdown');
    const time = sectionFields(OPTIONS, 'time', options).map(field => field.key);
    assert.equal(time.includes('playback'), view === 'lines');
    assert.equal(time.includes('windowDays'), view === 'lines');
    assert.equal(time.includes('afterPlayhead'), view === 'breakdown', 'the loss-history future switch stays usable independently of chart playback');
    assert.equal(sectionFields(OPTIONS, 'axes', options).some(field => field.key === 'yMax'), view === 'lines');
    assert.equal(options.dateOverlay, true, 'hidden settings retain their saved values');
    assert.equal(options.playback, 'window'); assert.equal(options.windowDays, 28); assert.equal(options.yMax, 500);
    const wholeCampaign = resolveOptions(OPTIONS, { view, playback: 'cursor' });
    assert.equal(sectionFields(OPTIONS, 'time', wholeCampaign).some(field => field.key === 'afterPlayhead'), view !== 'race');
  }
});
