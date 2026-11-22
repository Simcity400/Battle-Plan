import { gatedTime, heldCountry } from '../../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { drawBands } from '../../charts/draw.ts';
import type { PlotFrame } from '../../charts/frame';
import type { ChartTokens } from '../../charts/tokens';
import { registerWidgets } from '../../dashboard/definitions.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { readDrillCountry } from '../country-measure/drillCountry.ts';
import { NumericReply } from '../../dashboard/numericReply.ts';
import type { LossesReply } from '../../generated/dashboard';
import { definition } from './definition.ts';
import { lossAmount, lossItems, lossKey, lossesTotal, lossWindow, validateLossesReply } from './lossesData.ts';
import { lossHistory, lossNumbers, lossParts, lossRows, lossSegments, pinLosses, readLossSelection } from './lossDrill.ts';

function reply(count = 1): LossesReply {
  return { branchId: 'b', tag: 'opaque tag', ships: [], from: 100, to: 110, width: 64, exactHours: [], restarts: [], amountScale: 100000, retainedMonths: 6, warnings: [],
    landSource: 'combatLog', landLowerBound: null, destructionRuns: [{ firstHour: 100, lastHour: 110, countedFrom: 100, withheldFrom: null }],
    coverage: [{ firstHour: 100, lastHour: 110, coverage: 'logged' }],
    destructionCoverage: [{ firstHour: 100, lastHour: 110, reason: null }],
    designs: Array.from({ length: count }, (_, index) => ({ variant: index + 1, equipment: 'mod/item', archetype: 'mod/type', archetypeName: 'Mod type',
      names: [{ firstHour: 100, name: `Design ${index}`, known: true }] })),
    causes: [{ source: { kind: 'combatLog', reason: 0 }, domain: 'land', name: { key: 'mod/cause', template: 'Mod cause', text: 'Mod cause' },
      loss: true, unknown: null, unknownSpans: [], group: null,
      leaves: Array.from({ length: count }, (_, design) => ({ design, key: String(design), lowerBoundFrom: null,
        points: [{ hour: 100, totalRaw: '100000' }, { hour: 104, totalRaw: '150000' }, { hour: 108, totalRaw: '50000' }], exact: [] })) }] };
}
const window = { first: 100, hour: 110, end: 110, campaign: true, from: null };
const edgeAt = (history: ReturnType<typeof lossHistory>, hour: number) => history.model.edges[history.model.hours.indexOf(hour)];

test('inclusive range subtraction dates signed changes to their supplying reads, with no future backfill', () => {
  const data = reply(), leaf = data.causes[0].leaves[0];
  assert.equal(lossAmount(leaf, 107, 104), 50000n);
  assert.equal(lossAmount(leaf, 110, 104), -50000n);
  assert.equal(lossAmount(leaf, 103, 101), 0n);
  const items = lossItems(data, { ...window, first: 104, from: 104, hour: 107 });
  assert.equal(lossesTotal(items).exact, 50000n);
  assert.equal(items[1].final, -50000);
  assert.equal(items[1].weight, 50000);
  data.causes[0].leaves[0].points = [{ hour: 108, totalRaw: '300000' }];
  assert.equal(lossRows(data, { ...window, hour: 107 }).length, 0);
  assert.equal(lossAmount(leaf, 108, 104), 300000n);
});

test('only follow ignores its trailing frame start; explicit times refuse outside or reversed branch bounds', () => {
  const branch = { firstHour: 100, lastHour: 110 };
  assert.deepEqual(lossWindow({ firstHour: -50, lastHour: 105 }, branch, 'follow'),
    { first: 100, hour: 105, end: 110, campaign: false, from: null });
  assert.deepEqual(lossWindow({ firstHour: 104, lastHour: 108 }, branch, 'range'),
    { first: 104, hour: 108, end: 108, campaign: true, from: 104 });
  for (const mode of ['campaign', 'pin', 'range']) {
    assert.equal(lossWindow({ firstHour: 99, lastHour: 105 }, branch, mode), null);
    assert.equal(lossWindow({ firstHour: 109, lastHour: 105 }, branch, mode), null);
  }
  assert.equal(lossWindow({ firstHour: 100, lastHour: 111 }, branch, 'follow'), null);
  assert.equal(lossWindow(null, branch, 'campaign'), null);
});

test('signed and zero native rows remain reachable without bands, while stale reads keep their actual dates', () => {
  const data = reply(3), cause = data.causes[0];
  cause.leaves[0].points = [{ hour: 100, totalRaw: '-9007199254740993' }];
  cause.leaves[1].points = [{ hour: 104, totalRaw: '0' }];
  cause.leaves[2].points = [{ hour: 108, totalRaw: '100000' }];
  const rows = lossRows(data, { ...window, hour: 107 });
  assert.deepEqual(rows.map(row => [row.exact, row.readHour]), [[-9007199254740993n, 100], [0n, 104]]);
  assert.deepEqual(lossRows(data, window, new Set([rows[0].id])).map(row => row.exact), [-9007199254740993n]);
  const history = lossHistory(data, 100, 110, null, new Set([rows[0].id, rows[1].id]));
  assert.equal(history.model.edges.filter(Boolean).every(edges => edges!.every(edge => edge.low === 0 && edge.high === 0)), true);
  assert.equal(edgeAt(lossHistory(data, 100, 110), 108)![0].high, 0);
  assert.equal(edgeAt(lossHistory(data, 100, 110, null, new Set([lossKey(cause, data.designs[2])])), 108)![0].high, 1);
});

test('event history breaks unread and destruction intervals and resumes measured subtotals', () => {
  const data = reply();
  data.causes[0].unknownSpans = [{ firstHour: 105, lastHour: 106, reason: 'unread', detail: null }];
  const history = lossHistory(data, 100, 110);
  assert.equal(edgeAt(history, 105), null);
  assert.deepEqual(edgeAt(history, 107), [{ low: 0, high: 1.5 }]);
  assert.equal(history.model.breakBefore[history.model.hours.indexOf(107)], true);
  data.landSource = 'destruction'; data.causes[0].source = { kind: 'destruction', reason: 'unknown' };
  data.causes[0].unknownSpans = [];
  data.destructionCoverage = [{ firstHour: 100, lastHour: 103, reason: null }, { firstHour: 104, lastHour: 106, reason: 'Lost drain' },
    { firstHour: 107, lastHour: 110, reason: null }];
  const drain = lossHistory(data, 100, 110);
  assert.equal(edgeAt(drain, 104), null);
  assert.deepEqual(edgeAt(drain, 107), [{ low: 0, high: 1.5 }]);
  data.causes[0].unknownSpans = [{ firstHour: 104, lastHour: 110, reason: 'destructionGap', detail: 'Lost drain' }];
  const total = lossesTotal(lossItems(data, window));
  assert.equal(total.exact, 50000n); assert.equal(total.incomplete, true); assert.equal(total.finalUnknown, true);
});

test('an absent opening destruction drain leaves unknown edges and boolean gap markers until a supplying read', () => {
  const data = reply();
  data.landSource = 'destruction'; data.destructionRuns = [{ firstHour: 100, lastHour: 110, countedFrom: 104, withheldFrom: null }];
  data.causes[0].source = { kind: 'destruction', reason: 'unknown' };
  data.causes[0].leaves[0].points = [{ hour: 104, totalRaw: '150000' }];
  data.destructionCoverage = [{ firstHour: 104, lastHour: 110, reason: null }];
  const history = lossHistory(data, 100, 110);
  assert.equal(edgeAt(history, 100), null);
  assert.equal(history.model.breakBefore[history.model.hours.indexOf(100)], false);
  assert.deepEqual(edgeAt(history, 104), [{ low: 0, high: 1.5 }]);
  assert.equal(history.model.breakBefore[history.model.hours.indexOf(104)], true);
  assert.deepEqual(lossRows(data, { ...window, hour: 103 }), []);
  assert.deepEqual(lossRows(data, { ...window, hour: 104 }).map(row => [row.exact, row.readHour]), [[150000n, 104]]);

  data.from = 100; data.to = 210;
  data.destructionRuns = [
    { firstHour: 100, lastHour: 199, countedFrom: 104, withheldFrom: null },
    { firstHour: 200, lastHour: 210, countedFrom: 201, withheldFrom: null },
  ];
  data.destructionCoverage = [
    { firstHour: 104, lastHour: 199, reason: null },
    { firstHour: 201, lastHour: 210, reason: null },
  ];
  data.causes[0].leaves[0].points = [{ hour: 104, totalRaw: '500000' }, { hour: 201, totalRaw: '200000' }];
  assert.deepEqual(lossRows(data, { first: 100, hour: 200, end: 210, campaign: false, from: null }), []);
  assert.deepEqual(lossRows(data, { first: 100, hour: 201, end: 210, campaign: false, from: null }).map(row => [row.exact, row.readHour]), [[200000n, 201]]);
});

test('destruction history resets running totals and range baselines across recorded run boundaries', () => {
  const data = reply(2);
  data.from = 100;
  data.to = 250;
  data.landSource = 'destruction';
  data.destructionRuns = [
    { firstHour: 100, lastHour: 199, countedFrom: 101, withheldFrom: null },
    { firstHour: 200, lastHour: 250, countedFrom: 201, withheldFrom: null },
  ];
  data.destructionCoverage = [
    { firstHour: 101, lastHour: 199, reason: null },
    { firstHour: 201, lastHour: 250, reason: null },
  ];
  data.causes[0].source = { kind: 'destruction', reason: 'land_combat' };
  data.causes[0].leaves[0].points = [{ hour: 101, totalRaw: '500000' }, { hour: 201, totalRaw: '200000' }];
  data.causes[0].leaves[1].points = [{ hour: 101, totalRaw: '900000' }];

  const full = lossHistory(data, 100, 250);
  assert.deepEqual(edgeAt(full, 101), [{ low: 0, high: 14 }]);
  assert.equal(edgeAt(full, 200), null);
  assert.deepEqual(edgeAt(full, 201), [{ low: 0, high: 2 }]);
  assert.equal(full.model.breakBefore[full.model.hours.indexOf(201)], true);
  assert.equal(lossesTotal(lossItems(data, { first: 100, hour: 201, end: 250, campaign: false, from: null })).exact, 200000n);

  const ranged = lossHistory(data, 150, 250, 150);
  assert.deepEqual(edgeAt(ranged, 150), [{ low: 0, high: 0 }]);
  assert.equal(edgeAt(ranged, 200), null);
  assert.deepEqual(edgeAt(ranged, 201), [{ low: 0, high: 2 }]);

  data.destructionRuns = [
    { firstHour: 100, lastHour: 149, countedFrom: 100, withheldFrom: null },
    { firstHour: 150, lastHour: 199, countedFrom: 150, withheldFrom: null },
  ];
  data.destructionCoverage = [{ firstHour: 100, lastHour: 199, reason: null }];
  data.causes[0].leaves[0].points = [{ hour: 100, totalRaw: '500000' }, { hour: 150, totalRaw: '200000' }];
  data.causes[0].leaves[1].points = [{ hour: 100, totalRaw: '900000' }];
  const contiguous = lossHistory(data, 100, 199);
  assert.deepEqual(edgeAt(contiguous, 100), [{ low: 0, high: 14 }]);
  assert.deepEqual(edgeAt(contiguous, 150), [{ low: 0, high: 2 }]);
  assert.equal(contiguous.model.breakBefore[contiguous.model.hours.indexOf(150)], true);
});

test('exact read absence inside a multi-hour bin hides future loss at the read hour while keeping the bin loss on the chart', () => {
  const make = (exactHour: number) => {
    const data = reply(1);
    data.from = 120;
    data.to = 130;
    data.width = 1;
    data.exactHours = [exactHour];
    data.landSource = 'destruction';
    data.destructionRuns = [{ firstHour: 120, lastHour: 130, countedFrom: 121, withheldFrom: null }];
    data.destructionCoverage = [
      { firstHour: 121, lastHour: 123, reason: 'supplemental hooks unavailable' },
      { firstHour: 124, lastHour: 130, reason: null },
    ];
    data.causes[0].source = { kind: 'destruction', reason: 'land_combat' };
    data.causes[0].unknownSpans = [{ firstHour: 121, lastHour: 130, reason: 'destructionWithheld', detail: 'supplemental hooks unavailable' }];
    data.causes[0].leaves[0].points = [{ hour: 121, totalRaw: '100000' }];
    data.causes[0].leaves[0].exact = [{ hour: exactHour, totalRaw: null }];
    return data;
  };

  const at124 = make(124);
  assert.equal(lossRows(at124, { first: 120, hour: 124, end: 130, campaign: false, from: null }).every(row => row.exact === null), true);
  assert.equal(edgeAt(lossHistory(at124, 124, 124), 124), null);
  const chart124 = lossHistory(at124, 120, 130);
  assert.equal(edgeAt(chart124, 124), null);
  assert.deepEqual(edgeAt(chart124, 125), [{ low: 0, high: 1 }]);
  assert.equal(chart124.model.breakBefore[chart124.model.hours.indexOf(125)], true);

  const zero124 = make(124);
  zero124.causes[0].leaves[0].exact = [{ hour: 124, totalRaw: '0' }];
  assert.deepEqual(edgeAt(lossHistory(zero124, 124, 124), 124), [{ low: 0, high: 0 }]);

  const at121 = make(121);
  assert.equal(lossRows(at121, { first: 120, hour: 121, end: 130, campaign: false, from: null }).every(row => row.exact === null), true);
  const chart121 = lossHistory(at121, 120, 130);
  assert.equal(edgeAt(chart121, 121), null);
  assert.deepEqual(edgeAt(chart121, 124), [{ low: 0, high: 1 }]);
  assert.deepEqual(edgeAt(lossHistory(at121, 125, 125), 125), [{ low: 0, high: 1 }]);
  assert.deepEqual(edgeAt(lossHistory(at121, 130, 130), 130), [{ low: 0, high: 1 }]);
});

test('exact hover inside a rising multi-hour bin keeps the newer total without dropping back to the bin start', () => {
  const data = reply(1);
  data.from = 120;
  data.to = 125;
  data.width = 1;
  data.exactHours = [122];
  data.landSource = 'destruction';
  data.destructionRuns = [{ firstHour: 120, lastHour: 125, countedFrom: 120, withheldFrom: null }];
  data.destructionCoverage = [{ firstHour: 120, lastHour: 125, reason: null }];
  data.causes[0].source = { kind: 'destruction', reason: 'land_combat' };
  data.causes[0].leaves[0].points = [{ hour: 120, totalRaw: '100000' }, { hour: 125, totalRaw: '500000' }];
  data.causes[0].leaves[0].exact = [{ hour: 122, totalRaw: '500000' }];

  const chart = lossHistory(data, 120, 125);
  assert.deepEqual(edgeAt(chart, 120), [{ low: 0, high: 1 }]);
  assert.deepEqual(edgeAt(chart, 122), [{ low: 0, high: 5 }]);
  assert.equal(chart.model.hours.includes(123), false);
  assert.deepEqual(edgeAt(lossHistory(data, 123, 123), 123), [{ low: 0, high: 5 }]);
  assert.deepEqual(edgeAt(lossHistory(data, 125, 125), 125), [{ low: 0, high: 5 }]);

  const coarse = reply(1);
  coarse.from = 120;
  coarse.to = 130;
  coarse.width = 1;
  coarse.exactHours = [124];
  coarse.landSource = 'destruction';
  coarse.destructionRuns = [{ firstHour: 120, lastHour: 130, countedFrom: 121, withheldFrom: null }];
  coarse.destructionCoverage = [{ firstHour: 121, lastHour: 130, reason: null }];
  coarse.causes[0].source = { kind: 'destruction', reason: 'land_combat' };
  coarse.causes[0].leaves[0].points = [{ hour: 121, totalRaw: '100000' }, { hour: 125, totalRaw: '100000' }];
  coarse.causes[0].leaves[0].exact = [{ hour: 124, totalRaw: '0' }];
  const coarseChart = lossHistory(coarse, 120, 130);
  assert.deepEqual(edgeAt(coarseChart, 124), [{ low: 0, high: 0 }]);
  assert.deepEqual(edgeAt(coarseChart, 125), [{ low: 0, high: 1 }]);
  assert.deepEqual(edgeAt(lossHistory(coarse, 126, 126), 126), [{ low: 0, high: 1 }]);
  assert.deepEqual(lossRows(coarse, { first: 120, hour: 126, end: 130, campaign: false, from: null }).map(row => row.exact), [100000n]);
});

test('missing rows invent no amount or date, and exchange uncertainty belongs to its affected design', () => {
  const data = reply(2), cause = data.causes[0];
  cause.source = { kind: 'lendLeaseSunk' }; cause.domain = 'other';
  cause.unknownSpans = [{ firstHour: 108, lastHour: 110, reason: 'endedAfterLastRead', detail: null }];
  cause.leaves[0].lowerBoundFrom = 108;
  const rows = lossRows(data, window);
  assert.equal(rows[0].incomplete, true); assert.match(rows[0].note!, /Incomplete total/);
  assert.equal(rows[1].incomplete, false); assert.equal(rows[1].note, undefined);
  cause.leaves = [];
  assert.deepEqual(lossRows(data, window).map(row => [row.exact, row.readHour]), [[null, null]]);
  assert.equal(lossesTotal(lossItems(data, window)).exact, null);
});

test('defined empty names are preserved, and content-only updates retain the numeric pick model', () => {
  const data = reply(); data.causes[0].name.text = ''; data.designs[0].archetypeName = ''; data.designs[0].names[0].name = '';
  assert.equal(validateLossesReply(data), data);
  assert.deepEqual(lossRows(data, window).map(row => [row.cause, row.type, row.design]), [['', '', '']]);
  const cache = new NumericReply<LossesReply>(), numbers = cache.update('request', data, lossNumbers);
  const labels = structuredClone(data); labels.causes[0].name.text = 'Reloaded'; labels.designs[0].archetypeName = 'Reloaded'; labels.designs[0].names[0].name = 'Reloaded';
  assert.equal(cache.update('request', labels, lossNumbers), numbers);
  assert.equal(cache.update('request', undefined, lossNumbers), numbers);
  labels.causes[0].leaves[0].points[0].totalRaw = '999999';
  assert.notEqual(cache.update('request', { ...labels }, lossNumbers), numbers);
  assert.equal(cache.update('different recording', undefined, lossNumbers), undefined);
});

test('historical pins retain the country, native cohort and branch-relative hour; ranges retain their basis', () => {
  registerWidgets([definition]);
  const track = { id: 't', axisOffset: 1000, branch: { id: 'b', firstHour: 100, lastHour: 999 },
    source: { metadata: { campaignKey: 'campaign' } } } as BoundTrack;
  const widget: WidgetSpec = { id: 'w', kind: 'equipment-losses', configVersion: 1, title: 'Losses', campaign: 'tracks', trackIds: ['t'],
    countries: { kind: 'selected' }, time: { mode: 'campaign' }, options: { landSource: 'combatLog', lossSelection: 'old' } };
  const keys = [lossKey(reply().causes[0], reply().designs[0])];
  const country = { campaign: 'campaign', branch: 'b', tag: 'opaque tag', originOffset: 2, slot: 17, countryId: 21 };
  const held = heldCountry({ tag: 'opaque tag', origin: 102, slot: 17, countryId: 21 });
  const pinned = pinLosses(widget, track, held, gatedTime({ mode: 'pin', windows: { t: { firstOffset: 105, lastOffset: 105 } } }), keys);
  assert.deepEqual(readDrillCountry(pinned.options?.lossCountry, track, pinned.countries), country);
  assert.deepEqual(readDrillCountry(JSON.parse(JSON.stringify(pinned)).options.lossCountry, track, pinned.countries), country);
  assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['opaque tag'] });
  assert.deepEqual(pinned.time.value, { mode: 'pin', windows: { t: { firstOffset: 105, lastOffset: 105 } } });
  assert.equal(pinned.options?.show, 'loss-details');
  assert.deepEqual(readLossSelection(pinned.options?.lossSelection, 'campaign', 'b', 'opaque tag')?.keys, keys);
  for (const scope of [['another', 'b', 'opaque tag'], ['campaign', 'another', 'opaque tag'], ['campaign', 'b', 'another']])
    assert.equal(readLossSelection(pinned.options?.lossSelection, scope[0], scope[1], scope[2]), undefined);
  assert.equal(readLossSelection('malformed', 'campaign', 'b', 'opaque tag'), undefined);
  assert.deepEqual(pinLosses(widget, track, held, gatedTime({ mode: 'range', firstOffset: 50, lastOffset: 105 }), keys).time.value,
    { mode: 'range', firstOffset: 50, lastOffset: 105 });
  assert.deepEqual(pinLosses(widget, track, held, gatedTime({ mode: 'follow' }), null).time.value, { mode: 'follow' });
  const followed = pinLosses(widget, track, held, gatedTime({ mode: 'follow' }), null);
  assert.equal(followed.options?.lossSelection, undefined);
  assert.deepEqual(readDrillCountry(followed.options?.lossCountry, track, followed.countries), country);
});

test('all 5,000 native designs survive the signed inventory and native cohort history without a chosen cap', () => {
  const data = reply(5000);
  data.causes[0].leaves[4999].points[0].totalRaw = '-9007199254740993';
  const rows = lossRows(data, { ...window, hour: 100 });
  assert.equal(rows.length, 5000); assert.equal(new Set(rows.map(row => row.id)).size, 5000);
  assert.equal(rows[4999].exact, -9007199254740993n);
  const selected = new Set(rows.map(row => row.id));
  assert.equal(lossRows(data, { ...window, hour: 100 }, selected).length, 5000);
  assert.deepEqual(edgeAt(lossHistory(data, 100, 110, null, selected), 100), [{ low: 0, high: 0 }]);
  const parts = lossParts(rows, data.amountScale);
  assert.equal(parts.length, 1); assert.equal(parts[0].keys.length, 5000);
  assert.equal(parts[0].raw, 499900000n - 9007199254740993n);
  assert.equal(edgeAt(lossHistory(data, 100, 110, null, selected), 104)![0].high, 7500);
});


test('cause bars and history count the same signed cohort before positive geometry', () => {
  const data = reply(2);
  data.causes[0].leaves[0].points = [{ hour: 100, totalRaw: '300000' }];
  data.causes[0].leaves[1].points = [{ hour: 100, totalRaw: '-100000' }];
  const rows = lossRows(data, window), parts = lossParts(rows, data.amountScale);
  assert.equal(parts[0].raw, 200000n); assert.equal(parts[0].value, 2);
  assert.deepEqual(parts[0].keys, rows.map(row => row.id));
  assert.deepEqual(edgeAt(lossHistory(data, 100, 110), 100), [{ low: 0, high: parts[0].value! }]);
  assert.deepEqual(rows.map(row => row.exact), [300000n, -100000n]);
});


test('the reached painter holds clipped and thinned samples until the supplying hour and keeps gaps separate', () => {
  const data = reply(), history = lossHistory(data, 100, 110);
  const clipped = lossSegments(history.model, 102, 107, 0)[0];
  const vertices: [number, number][] = [];
  const context = { beginPath() {}, closePath() {}, fill() {}, stroke() {},
    moveTo(x: number, y: number) { vertices.push([x, y]); }, lineTo(x: number, y: number) { vertices.push([x, y]); } } as unknown as CanvasRenderingContext2D;
  drawBands(context, { toX: (hour: number) => hour, toY: (value: number) => value } as PlotFrame,
    { surface: '#000000' } as ChartTokens, clipped.hours, clipped.bands, ['#ffffff'], clipped);
  assert.deepEqual(vertices.slice(0, clipped.hours.length), [[102, 1], [104, 1], [104, 1.5], [107, 1.5], [107, 1.5]]);
  const thinned = lossSegments(history.model, 100, 110, 999)[0];
  assert.ok(thinned.hours.some((hour, index) => hour === 104 && thinned.bands[0].high[index] === 1.5));
  for (const piece of [clipped, thinned]) for (let index = 1; index < piece.hours.length; index++)
    if (piece.hours[index] > piece.hours[index - 1]) assert.equal(piece.bands[0].high[index], piece.bands[0].high[index - 1]);
  data.causes[0].unknownSpans = [{ firstHour: 105, lastHour: 106, reason: 'unread', detail: null }];
  const pieces = lossSegments(lossHistory(data, 100, 110).model, 100, 110, 999);
  assert.equal(pieces.length, 2);
  assert.equal(pieces[0].hours.at(-1), 105); assert.equal(pieces[1].hours[0], 107);
  assert.equal(pieces[0].bands[0].high.at(-1), 1.5);
  assert.deepEqual(lossSegments(lossHistory(data, 100, 110).model, 104.5, 110, 999)[0].hours, [104.5, 105, 105]);
  data.causes[0].unknownSpans = [];
  const identityGap = lossHistory(data, 100, 110, null, null, [{ firstHour: 100, lastHour: 104 }, { firstHour: 107, lastHour: 110 }]);
  assert.equal(edgeAt(identityGap, 105), null, 'a complete loss table cannot fill an unknown native identity');
  assert.equal(lossSegments(identityGap.model, 100, 110, 999).length, 2);
});
