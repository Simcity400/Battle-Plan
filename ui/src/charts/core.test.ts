import test from 'node:test';
import assert from 'node:assert/strict';
import { disambiguateLabels, drawFittedLabel, ellipsise, fitLabel, fitLabelWidth, fitLine, labelFitsTwoLines, labelLineHeight, layoutEndLabels } from './labels.ts';
import { runAt } from './series.ts';
import type { Run } from './series.ts';
import { backingSize } from './surface.ts';
import { placeTooltip } from './tooltipPlacement.ts';
import { fixedNumber, formatFixed, linearTicks, logTicks, niceStep } from './valueAxis.ts';

const RUNS: Run[] = [
  { firstHour: 10, lastHour: 19, value: 5 },
  { firstHour: 20, lastHour: 29, value: 8 },
  { firstHour: 40, lastHour: 49, value: 2 },
];

test('runs answer by hour and gaps stay missing, never zero', () => {
  assert.equal(runAt(RUNS, 15)?.value ?? null, 5);
  assert.equal(runAt(RUNS, 29)?.value ?? null, 8);
  assert.equal(runAt(RUNS, 35)?.value ?? null, null);
  assert.equal(runAt(RUNS, 9), null);
});

test('value ticks are nice and fixed-point values format exactly', () => {
  assert.equal(niceStep(100), 20);
  assert.equal(niceStep(7, 5), 2);
  assert.deepEqual(linearTicks(0, 100), [0, 20, 40, 60, 80, 100]);
  assert.deepEqual(linearTicks(-0.3, 0.3, 3), [-0.2, 0, 0.2]);
  assert.deepEqual(logTicks(1, 100), [1, 2, 5, 10, 20, 50, 100]);
  assert.deepEqual(logTicks(0, 100), []);
  assert.equal(formatFixed('1000001', 5, 2), '10.00');
  assert.equal(formatFixed('1234567890', 5, 1), '12,345.7');
  assert.equal(formatFixed('-150000', 5, 0), '−2');
  assert.equal(formatFixed('-49', 5, 2), '0.00');
  assert.equal(formatFixed(12n, 0, 2), '12.00');
  assert.equal(fixedNumber('250000', 5), 2.5);
});

test('end labels keep their order without overlapping and stay in bounds', () => {
  const placed = layoutEndLabels([{ id: 'a', y: 50, height: 14 }, { id: 'b', y: 52, height: 14 }, { id: 'c', y: 198, height: 14 }], 0, 200);
  assert.ok(placed.get('b')! - placed.get('a')! >= 14);
  assert.ok(placed.get('c')! <= 193);
  assert.equal(fitLabelWidth(['ab', 'abcd'], text => text.length * 10, 20, 200, 6), 46);
  assert.equal(fitLabelWidth(['x'.repeat(100)], text => text.length * 10, 20, 200), 200);
  const measured: string[] = [];
  const width = fitLabelWidth(['ab', 'x'.repeat(30), 'abcd', 'y'.repeat(40)], text => { measured.push(text); return text.length * 10; }, 20, 200, 6);
  assert.equal(width, 200);
  assert.deepEqual(measured, ['ab', 'x'.repeat(30)]);
});

test('canvases follow on-screen size, so the Size zoom stays sharp', () => {
  const at200 = backingSize({ layoutWidth: 300, layoutHeight: 100, rectWidth: 600, rectHeight: 200, devicePixelRatio: 1.5 });
  assert.deepEqual(at200, { width: 900, height: 300, scaleX: 3, scaleY: 3 });
  const exact = backingSize({ layoutWidth: 300, layoutHeight: 100, rectWidth: 600, rectHeight: 200, devicePixelRatio: 1.5, devicePixelWidth: 901, devicePixelHeight: 299 });
  assert.equal(exact.width, 901);
});

test('tooltips flip away from the window edge and are clamped inside it', () => {
  const viewport = { width: 1000, height: 800 };
  assert.deepEqual(placeTooltip({ x: 100, y: 100 }, { width: 200, height: 100 }, viewport), { x: 114, y: 114 });
  assert.deepEqual(placeTooltip({ x: 950, y: 750 }, { width: 200, height: 100 }, viewport), { x: 736, y: 636 });
  assert.deepEqual(placeTooltip({ x: 10, y: 10 }, { width: 2000, height: 100 }, viewport).x, 8);
});

test('text fitter wraps two balanced lines at word boundaries, cuts with end ellipsis only, and handles CJK, SEA, RTL, emoji, and tight bounds', () => {
  const mono = (text: string) => [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(text)].length * 8;

  const single = fitLabel('Infantry Equipment', 200, mono, { maxLines: 2 });
  assert.deepEqual(single, { lines: ['Infantry Equipment'], truncated: false, full: 'Infantry Equipment' });

  const balanced = fitLabel('Medium Tank Chassis 1940', 112, mono, { maxLines: 2 });
  assert.equal(balanced.truncated, false);
  assert.equal(balanced.lines.length, 2);
  assert.equal(balanced.lines.join(' '), 'Medium Tank Chassis 1940');
  assert.ok(Math.abs(balanced.lines[0]!.length - balanced.lines[1]!.length) <= 4);

  const cutTwo = fitLabel('Super Heavy Self Propelled Anti Aircraft Artillery', 100, mono, { maxLines: 2 });
  assert.equal(cutTwo.truncated, true);
  assert.equal(cutTwo.full, 'Super Heavy Self Propelled Anti Aircraft Artillery');
  assert.equal(cutTwo.lines.length, 2);
  assert.equal(cutTwo.lines[1]!.endsWith('\u2026'), true);
  assert.equal(cutTwo.lines[1]!.startsWith('\u2026'), false);
  assert.ok(mono(cutTwo.lines[0]!) <= 100 && mono(cutTwo.lines[1]!) <= 100);

  const longWord = fitLabel('Supercalifragilisticexpialidocious', 80, mono, { maxLines: 2 });
  assert.equal(longWord.truncated, true);
  assert.equal(longWord.lines.length, 1);
  assert.equal(longWord.lines[0]!.endsWith('\u2026'), true);
  assert.ok(mono(longWord.lines[0]!) <= 80);

  const cjk = fitLabel('大日本帝国陸軍省特別攻撃隊', 64, mono, { maxLines: 2 });
  assert.equal(cjk.lines.length, 2);
  assert.equal(cjk.full, '大日本帝国陸軍省特別攻撃隊');
  assert.ok(mono(cjk.lines[0]!) <= 64 && mono(cjk.lines[1]!) <= 64);

  const korean = fitLabel('대한민국임시정부광복군', 56, mono, { maxLines: 2 });
  assert.equal(korean.lines.length, 2);
  assert.ok(mono(korean.lines[0]!) <= 56 && mono(korean.lines[1]!) <= 56);

  const kinsoku = fitLabel('日本帝国。陸軍', 40, mono, { maxLines: 2 });
  assert.equal(kinsoku.lines[1]!.startsWith('。'), false);

  const thai = fitLabel('กองทัพบกไทยและกองทัพเรือ', 96, mono, { maxLines: 2 });
  assert.equal(thai.lines.length, 2);
  assert.ok(mono(thai.lines[0]!) <= 96 && mono(thai.lines[1]!) <= 96);

  const rtl = fitLabel('المملكة العربية السعودية المتحدة', 120, mono, { maxLines: 2 });
  assert.equal(rtl.lines.length, 2);
  assert.ok(mono(rtl.lines[0]!) <= 120 && mono(rtl.lines[1]!) <= 120);

  const emoji = fitLine('👨‍👩‍👧‍👦🇺🇸abc', 24, mono);
  assert.equal(emoji.truncated, true);
  assert.equal(emoji.text, '👨‍👩‍👧‍👦🇺🇸\u2026');
  assert.equal(ellipsise('Hello', 0, mono), '');
  assert.equal(ellipsise('Hello', 8, mono), '\u2026');

  assert.equal(labelFitsTwoLines(28, labelLineHeight(12)), false);
  assert.equal(labelFitsTwoLines(36, labelLineHeight(12)), true);

  const drawn: { text: string; x: number; y: number }[] = [];
  const fakeCtx = {
    font: '12px sans-serif',
    textAlign: 'left',
    textBaseline: 'middle',
    measureText: (t: string) => ({ width: mono(t) }),
    fillText: (text: string, x: number, y: number) => { drawn.push({ text, x, y }); },
  };
  const rendered = drawFittedLabel(fakeCtx as never, 'Long Division Template Name Here', 10, 20, 80, { fontSize: 12, lineHeight: 15, maxLines: 2 });
  assert.equal(rendered.truncated, true);
  assert.equal(rendered.full, 'Long Division Template Name Here');
  assert.equal(drawn.length, 2);
  assert.equal(drawn[1]!.y - drawn[0]!.y, 15);
});

test('disambiguateLabels distinguishes colliding shared-prefix labels without splitting joined graphemes', async () => {
  const mono = (text: string) => Array.from(new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(text)).length * 8;
  const siblings = [
    'Federation of Allied Northern States Alpha',
    'Federation of Allied Northern States Beta',
  ];
  const resolved = disambiguateLabels(siblings, 128, mono);
  assert.equal(resolved.length, 2);
  assert.notEqual(resolved[0], resolved[1], 'colliding siblings become distinct');
  assert.ok(resolved[0]!.endsWith('Alpha') && resolved[1]!.endsWith('Beta'));
  assert.ok(mono(resolved[0]!) <= 128 && mono(resolved[1]!) <= 128);

  const joined = [
    '👨‍👩‍👧‍👦🇺🇸 Northern Coalition Division East',
    '👨‍👩‍👧‍👦🇺🇸 Northern Coalition Division West',
  ];
  const resolvedJoined = disambiguateLabels(joined, 112, mono);
  assert.notEqual(resolvedJoined[0], resolvedJoined[1]);
  assert.ok(resolvedJoined[0]!.startsWith('👨‍👩‍👧‍👦🇺🇸') && resolvedJoined[0]!.endsWith('East'));
  assert.ok(resolvedJoined[1]!.endsWith('West'));

  const identical = ['Long Repeated Country Name One', 'Long Repeated Country Name One'];
  const same = disambiguateLabels(identical, 96, mono);
  assert.equal(same[0], ellipsise(identical[0]!, 96, mono));
  assert.equal(same[0], same[1]);

  const middleDiff = disambiguateLabels(
    ['Federation of Northern Coastal Republics', 'Federation of Southern Coastal Republics'],
    112,
    mono,
  );
  assert.notEqual(middleDiff[0], middleDiff[1], 'shared prefix and shared suffix still preserve the differing middle word');
  assert.ok(middleDiff[0]!.includes('Northern') && middleDiff[1]!.includes('Southern'));
  assert.ok(mono(middleDiff[0]!) <= 112 && mono(middleDiff[1]!) <= 112);

  const crossGroupNames = [
    'Northern Alliance East',
    'Northern Alliance West',
    'Northern Assembly East',
    'Northern Assembly West',
  ];
  const nestedOverlapNames = [
    'Federation of Northern Alliance East',
    'Federation of Northern Alliance West',
    'Federation of Southern Alliance East',
  ];
  for (const width of [96, 112]) {
    const crossGroup = disambiguateLabels(crossGroupNames, width, mono);
    assert.equal(new Set(crossGroup).size, 4, `cross-group collisions at ${width}px converge to 4 distinct labels (${crossGroup.join(', ')})`);
    assert.ok(crossGroup.every(label => mono(label) <= width), `all labels fit within ${width}px (${crossGroup.join(', ')})`);

    const nestedOverlap = disambiguateLabels(nestedOverlapNames, width, mono);
    assert.equal(new Set(nestedOverlap).size, 3, `overlapping spans at ${width}px stay distinct (${nestedOverlap.join(', ')})`);
    assert.ok(nestedOverlap.every(label => mono(label) <= width), `overlapping labels fit within ${width}px (${nestedOverlap.join(', ')})`);
  }

  const { layoutFamilyBars } = await import('../families/bars/layout.ts');
  const { drawFamilyBars } = await import('../families/bars/painter.ts');
  const { knownValue } = await import('../families/value.ts');
  const { NO_ART } = await import('../families/art/types.ts');
  const splitModel = {
    form: 'two-sided' as const,
    twoSidedLayout: 'split-sides' as const,
    hour: 200,
    rows: [
      {
        kind: 'data' as const,
        key: 'L1',
        subject: 'L1',
        label: knownValue('Federation of Allied Northern Coastal Republics'),
        media: { kind: 'none' as const, art: NO_ART },
        color: '#3987e5',
        value: { value: knownValue(50), format: 'integer' as const },
        numeric: 50,
        rightSide: {
          key: 'R1',
          subject: 'R1',
          label: knownValue('Northern Coastal Coalition 1'),
          media: { kind: 'none' as const, art: NO_ART },
          color: '#d95926',
          value: { value: knownValue(60), format: 'integer' as const },
          numeric: 60,
        },
      },
      {
        kind: 'data' as const,
        key: 'L2',
        subject: 'L2',
        label: knownValue('Federation of Allied Southern Coastal Republics'),
        media: { kind: 'none' as const, art: NO_ART },
        color: '#3987e5',
        value: { value: knownValue(40), format: 'integer' as const },
        numeric: 40,
        rightSide: {
          key: 'R2',
          subject: 'R2',
          label: knownValue('Northern Coastal Coalition 2'),
          media: { kind: 'none' as const, art: NO_ART },
          color: '#d95926',
          value: { value: knownValue(30), format: 'integer' as const },
          numeric: 30,
        },
      },
    ],
  };
  const barsTheme = {
    surface: '#111111', raised: '#1b1b1b', text: '#f0f0f0', muted: '#999999', line: '#333333',
    accent: '#3987e5', green: '#199e70', grey: '#666666', dark: true, font: 'sans-serif',
    fontSize: { small: 11, body: 12 },
  };
  const barsLayout = layoutFamilyBars(splitModel, 720, 120, barsTheme as never, text => text.length * 9);
  assert.equal(barsLayout.twoSidedLayout, 'split-sides');
  const barTexts: string[] = [];
  const barCtx = {
    font: '12px sans-serif',
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    globalAlpha: 1,
    textAlign: 'left',
    textBaseline: 'middle',
    measureText(t: string) { return { width: t.length * (this.font.startsWith('500') ? 9 : 7) }; },
    save() {},
    restore() {},
    clearRect() {},
    beginPath() {},
    rect() {},
    roundRect() {},
    clip() {},
    moveTo() {},
    lineTo() {},
    stroke() {},
    fill() {},
    fillRect() {},
    fillText(t: string) { barTexts.push(t); },
  };
  drawFamilyBars(barCtx as never, {
    model: splitModel,
    layout: barsLayout,
    theme: barsTheme as never,
    scrollTop: 0,
    now: 0,
    glideDuration: 0,
    glide: new Map(),
    inspectedKey: null,
    selectedKey: null,
    focusedKey: null,
  });
  assert.ok(barTexts.some(t => t.startsWith('Federation') && t.includes('Northern')) && barTexts.some(t => t.startsWith('Federation') && t.includes('Southern')), `left split-side bars disambiguate (${barTexts.join(', ')})`);
  assert.ok(barTexts.some(t => t.endsWith('1')) && barTexts.some(t => t.endsWith('2')), `right split-side bars fit with label weight and disambiguate (${barTexts.join(', ')})`);

  const { layoutTimeChart } = await import('../families/time-chart/layout.ts');
  const tcLayout = layoutTimeChart(
    {
      kind: 'test',
      measureTitle: 'Test',
      mode: 'lines',
      hour: 200,
      extent: [100, 200],
      yAxis: { format: 'number' },
      series: [
        {
          id: 'a',
          key: 'a',
          name: 'Federation of Allied Northern States Alpha',
          color: '#3987e5',
          role: 'primary',
          headValue: 10,
          headHour: 200,
          points: [{ hour: 100, value: 10 }, { hour: 200, value: 10 }],
          endLabel: { name: 'Federation of Allied Northern States Alpha', fullName: 'Federation of Allied Northern States Alpha', value: '10' },
        },
        {
          id: 'b',
          key: 'b',
          name: 'Federation of Allied Northern States Beta',
          color: '#d95926',
          role: 'primary',
          headValue: 20,
          headHour: 200,
          points: [{ hour: 100, value: 20 }, { hour: 200, value: 20 }],
          endLabel: { name: 'Federation of Allied Northern States Beta', fullName: 'Federation of Allied Northern States Beta', value: '20' },
        },
      ],
    },
    360,
    200,
    'sans-serif',
    mono,
  );
  const endNames = tcLayout.endLabels.map(label => label.name).filter((n): n is string => n !== null);
  assert.equal(endNames.length, 2);
  assert.notEqual(endNames[0], endNames[1], 'time-chart end labels disambiguate colliding shared-prefix names');
});
