import assert from 'node:assert/strict';
import test from 'node:test';
import { fitTooltip, tooltipHeight, tooltipRoom, tooltipRowHeight, tooltipText } from './tooltip.ts';
import type { TooltipContent, TooltipRow } from './tooltip';

const rows = (count: number, prefix = 'Row'): TooltipRow[] => Array.from({ length: count }, (_, index) => ({ label: `${prefix} ${index + 1}`, value: String(index) }));
const total: TooltipRow = { label: 'Total', value: '99', strong: true, divider: true };

test('the fullest form that fits is shown, built only as far as needed', () => {
  let built = 0;
  function* forms(): Generator<TooltipContent> {
    for (const count of [40, 20, 10]) { built++; yield { title: 'T', rows: [...rows(count), total] }; }
    built++;
    yield { title: 'T', rows: [total] };
  }
  const tall = fitTooltip(forms(), 1200);
  assert.equal(tall.rows.length, 41);
  assert.equal(built, 1);
  assert.equal(tall.spoken, undefined);
  built = 0;
  const medium = fitTooltip(forms(), 500);
  assert.ok(tooltipHeight(medium) <= tooltipRoom(500));
  assert.equal(medium.rows.length, 21);
  assert.equal(built, 2);
  assert.equal(tooltipText(medium), tooltipText({ title: 'T', rows: [...rows(40), total] }));
});

test('when nothing fits, the last form is cut with its totals kept and the rest counted', () => {
  const long: TooltipContent = { title: 'T', subtitle: 'A line', icon: 'icon', rows: [...rows(60), total], note: 'Read earlier; values are read once a day.' };
  for (const height of [120, 160, 200, 300, 450, 700]) {
    const tip = fitTooltip([long], height, { keep: 1 });
    assert.ok(tooltipHeight(tip) <= tooltipRoom(height) || tip.rows.length === 2, `${height}: ${tooltipHeight(tip)}`);
    assert.deepEqual(tip.rows.at(-1), total, `${height}`);
    const more = tip.rows.at(-2)!;
    const hidden = Number(/^(\d+) more$/.exec(more.label)?.[1]);
    assert.equal(tip.rows.length - 2 + hidden, 60, `${height}`);
    assert.equal(tip.spoken, tooltipText(long));
  }
  assert.equal(fitTooltip([long], 120, { keep: 1 }).note, undefined);
  assert.equal(fitTooltip([long], 700, { keep: 1 }).note, long.note);
  assert.equal(fitTooltip([{ title: 'T', rows: rows(80) }], 400, { more: hidden => `${hidden} more resources` }).rows.at(-1)!.label.endsWith('more resources'), true);
});

test('wrapping labels and dividers count toward the height', () => {
  const short = tooltipHeight({ title: 'T', rows: [{ label: 'Label', value: '1' }] });
  const wrapped = tooltipHeight({ title: 'T', rows: [{ label: 'A label long enough to wrap onto a second line in the tooltip', value: '1' }] });
  const kept = tooltipHeight({ title: 'T', rows: [{ label: 'A label long enough to wrap onto a second line in the tooltip', value: '1', short: true }] });
  assert.ok(wrapped > short);
  assert.equal(kept, short);
  assert.ok(tooltipHeight({ title: 'T', rows: [{ label: 'Label', value: '1', divider: true }] }) > short);
});


test('a full form, when given, is what assistive technology reads whatever is shown', () => {
  const full: TooltipContent = { title: 'Rifles', rows: Array.from({ length: 10 }, (_, index) => ({ label: `Variant ${index}`, value: String(index) })) };
  const capped: TooltipContent = { title: 'Rifles', rows: [...full.rows.slice(0, 8), { label: '2 more variants' }] };
  const shown = fitTooltip([capped], 10000, { full });
  assert.equal(shown.rows.length, 9);
  assert.ok(shown.spoken?.includes('Variant 9'));
  assert.ok(fitTooltip([capped], 60, { full }).spoken?.includes('Variant 9'));
  assert.equal(fitTooltip([capped], 10000).spoken, undefined);
});

test('missing flags add no tooltip rows, spoken text or fitting height', () => {
  const content: TooltipContent = { title: 'Pending country', countryFlag: { flagPending: true },
    rows: [1, 2, 3].map(index => ({ label: `A long recorded country name ${index}`, value: '1', countryFlag: {} })) };
  const fitted = fitTooltip([content], 160);
  assert.equal(fitted.rows.length, content.rows.length, 'missing flags do not consume room for explanations');
  assert.ok(tooltipHeight(fitted) <= tooltipRoom(160));
  assert.doesNotMatch(tooltipText(fitted), /Flag loading/);
  assert.doesNotMatch(tooltipText(fitted), /Flag not available|Flag unavailable/);
  assert.equal(tooltipHeight(content), tooltipHeight({ ...content, countryFlag: { flag: 'synthetic.png' } }));
  const row = { label: 'Member', value: '5' };
  assert.equal(tooltipRowHeight({ ...row, countryFlag: { flagReason: 'Synthetic native refusal that is long enough to wrap' } }), tooltipRowHeight(row));
});

test('refitting missing flags in a smaller window preserves original omissions and the kept total', () => {
  const countries: TooltipRow[] = rows(5000, 'Native country').map((row, index) => ({ ...row, countryFlag: { flag: `synthetic-flag-${index}` } }));
  countries[0] = { ...countries[0], omittedRows: -1 };
  const raw: TooltipContent = { title: 'Native countries', countryFlag: { flag: 'synthetic-header' }, rows: [...countries, total] };
  const more = (hidden: number) => `${hidden} more participants`;
  const first = fitTooltip([raw], 360, { keep: 1, more });
  const firstSummary = first.rows.find(row => row.tone === 'muted')!;
  const firstVisible = first.rows.filter(row => row.countryFlag).length;
  const originallyOmitted = countries.length - firstVisible;
  assert.equal(firstSummary.label, more(originallyOmitted));
  assert.equal(first.rows[0], countries[0]);
  assert.equal(first.rows.at(-1), total);
  assert.equal(first.spoken, tooltipText(raw));

  const failure = { flagReason: 'Synthetic displayed image decoding was refused with a detailed recorded explanation', flagPending: false };
  let decoded = 0;
  const displayed: TooltipContent = { ...first, countryFlag: failure, rows: first.rows.map(row => {
    if (!row.countryFlag) return row;
    decoded++;
    return { ...row, countryFlag: failure };
  }) };
  assert.equal(decoded, firstVisible, 'only the initially displayed country rows were decoded');
  const unchanged = fitTooltip([displayed], 360, { more });
  assert.equal(unchanged.rows.filter(row => row.countryFlag).length, firstVisible, 'flag failures add no explanatory rows');
  const final = fitTooltip([displayed], 260, { more });
  const summary = final.rows.find(row => row.tone === 'muted')!;
  const visible = final.rows.filter(row => row.countryFlag).length;
  assert.ok(visible < firstVisible, 'the smaller window hides additional real rows');
  assert.equal(summary.label, more(countries.length - visible), 'the displayed count includes old and newly hidden real rows');
  assert.equal(summary.omittedRows, originallyOmitted + firstVisible - visible);
  assert.equal(firstSummary.omittedRows, originallyOmitted);
  assert.equal(visible + summary.omittedRows!, countries.length);
  assert.equal(final.rows.filter(row => (row.omittedRows ?? 0) > 0).length, 1);
  assert.equal(final.rows.at(-1), total, 'a second fit inherits the previously kept suffix');
  assert.equal(final.spoken, first.spoken);
  assert.equal(tooltipText(final), tooltipText(raw));
  assert.ok(tooltipHeight(final) <= tooltipRoom(260));
});

test('rows wrap by their width in the active font, so a wide font cuts rows the character estimate would keep', () => {
  const metrics = (perChar: number) => ({ measure: (text: string) => text.length * perChar, family: 'Test Sans', sizes: { xs: 11, sm: 12, md: 13 } });
  const narrow = metrics(6);
  const wide = metrics(14);
  const row: TooltipRow = { label: 'Wide label of twenty', value: '1,234' };
  assert.equal(tooltipRowHeight(row, narrow), 17, 'one line in a narrow font');
  assert.equal(tooltipRowHeight(row, wide), 34, 'the same row wraps to two lines in a wide font');
  const content: TooltipContent = { title: 'T', rows: Array.from({ length: 12 }, () => row) };
  const height = tooltipHeight(content, narrow) + 16;
  assert.equal(fitTooltip([content], height, { metrics: narrow }).rows.length, 12, 'everything fits in the narrow font');
  const cut = fitTooltip([content], height, { metrics: wide });
  assert.ok(tooltipHeight(cut, wide) <= tooltipRoom(height), 'the wide font is cut to the room it really needs');
  assert.ok(cut.rows.length < 12);
  assert.equal(cut.spoken, tooltipText(content), 'assistive text keeps every row');

  const even = metrics(7);
  const flagged: TooltipRow = { label: 'Federal Republic of Neverland', value: '11,000,000', countryFlag: {} };
  assert.equal(tooltipRowHeight({ ...flagged, countryFlag: undefined }, even), 17, 'label, gap and value fit the row');
  assert.equal(tooltipRowHeight(flagged, even), 34, 'the flag and the gap between label and value take the room the text needs, so the row wraps');
  assert.equal(tooltipRowHeight({ label: 'A', value: '1'.repeat(40) }, even), 34, 'a value wider than its share wraps even beside a short label');
  assert.equal(tooltipRowHeight({ label: 'A', value: '1'.repeat(39) }, metrics(9)), 51, 'an unbroken value wraps where its glyphs stop fitting: 19, 19 and 1');
  const larger = { ...metrics(6), sizes: { xs: 13, sm: 15, md: 16 } };
  assert.equal(tooltipRowHeight({ label: 'A', value: '1' }, larger), 21, 'a larger interface type has taller lines');
  const title = 'The Kingdom of the Netherlands in exile';
  assert.equal(tooltipHeight({ title, rows: [], countryFlag: {} }, even), tooltipHeight({ title, rows: [] }, even) + 19, 'a flagged title has the room beside its flag, and wraps there');
});
