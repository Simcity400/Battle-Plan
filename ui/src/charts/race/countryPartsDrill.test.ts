import assert from 'node:assert/strict';
import test from 'node:test';
import { createWidget } from '../../dashboard/model.ts';
import type { BoundTrack } from '../../dashboard/model';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill } from './partsRaceDrill.ts';
import { OPTIONS as FACTORY_OPTIONS } from '../../widgets/factories/definition.ts';
import { OPTIONS as PRODUCED_OPTIONS } from '../../widgets/equipment-produced/definition.ts';
import '../../widgets/definitions.ts';
import { pinCountryParts } from './countryPartsDrillData.ts';

test('both economy widgets expose full drill settings and pin the clicked country and branch-relative hour', () => {
  const track = { id: 'track', branch: { firstHour: 1000 } } as Pick<BoundTrack, 'id' | 'branch'>;
  for (const [kind, options] of [['factories', FACTORY_OPTIONS], ['equipment-produced', PRODUCED_OPTIONS]] as const) {
    const source = { ...createWidget(kind, track.id), countries: { kind: 'group' as const, group: { type: 'top' as const, count: 10 }, membership: 'hour' as const, aggregate: false },
      time: { mode: 'follow' as const }, options: { drill: 'popout', grouping: 'variant', count: 'built' } };
    const pinned = pinCountryParts(source, track, 'mod country, exact', 1051.9, 1000, false, 'Breakdown');
    assert.equal(pinned.kind, kind);
    assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['mod country, exact'] });
    assert.deepEqual(pinned.time, { mode: 'pin', windows: { track: { firstOffset: 51, lastOffset: 51 } } });
    assert.equal(pinned.options?.show, 'breakdown');
    assert.equal(pinned.options?.drillOriginHour, 1000);
    const held = resolveOptions(options, pinned.options);
    assert.equal(held.drill, 'popout');
    assert.ok(sectionFields(options, 'time', held).some(field => field.key === 'playback'));
    assert.equal(resolveOptions(options, undefined).show, 'main');
    assert.equal(resolveOptions(options, undefined).drill, 'inline');
    const follow = pinCountryParts(source, track, 'mod country, exact', 1100, 1000, true, 'Breakdown');
    assert.equal(follow.time.mode, 'follow');
    assert.equal(follow.options?.drillOriginHour, 1000);
  }
});

test('overlapping drawn actors keep the clicked hour, refuse stale input, and resume playback only when requested', () => {
  const actors = partsRaceDrillActors(undefined, ['MOD A', 'MOD B'], () => 'a later race winner');
  const waiting = reducePartsRaceDrill(CLOSED_PARTS_RACE, { type: 'actors', key: 'clicked', tags: actors, hour: 50 });
  assert.equal(waiting.breakdown, null);
  assert.equal(waiting.hour, 50);
  assert.equal(reducePartsRaceDrill(waiting, { type: 'choose', key: 'stale', tag: 'MOD A', hour: 50 }), waiting);
  const picked = reducePartsRaceDrill(waiting, { type: 'choose', key: 'clicked', tag: 'MOD B', hour: 50 });
  assert.equal(picked.breakdown, 'MOD B');
  assert.equal(picked.follow, false);
  assert.equal(picked.hour, 50);
  const following = reducePartsRaceDrill(picked, { type: 'open', breakdown: 'MOD B', hour: 50, playing: true, playhead: 5000 });
  assert.equal(following.follow, true);
  assert.equal(following.hour, null);
  assert.deepEqual(partsRaceDrillActors(undefined, [], () => 'a later race winner'), []);
});
