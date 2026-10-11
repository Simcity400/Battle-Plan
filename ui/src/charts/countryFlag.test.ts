import assert from 'node:assert/strict';
import test from 'node:test';
import { countryFlagUnavailable } from '../components/countryFlagPresentation.ts';
import { canvasCountryFlag, countryFlagContent, countryTooltipContent, drawCountryFlag } from './countryFlag.ts';
import type { TooltipContent } from './tooltip';

const tokens = { surface: '#000000', muted: '#888888', border: '#444444', lineWidth: { grid: 1, series: 2, emphasis: 3 } };

test('genuine missing flags keep a filled slot and their exact read reason; flagless art stays flagless', () => {
  const calls: string[] = [];
  const context = { save() {}, restore() {}, fillRect() { calls.push('fill'); }, strokeRect() { calls.push('outline'); },
    drawImage() { calls.push('image'); } } as unknown as CanvasRenderingContext2D;
  const wake = () => {};
  const missing = { flagReason: 'Synthetic mod refused the recorded flag definition.', flagPending: false };
  drawCountryFlag(context, undefined, 0, 0, 18, 12, tokens, wake);
  assert.deepEqual(calls, [], 'intentionally flagless entity has no placeholder');
  drawCountryFlag(context, missing, 0, 0, 18, 12, tokens, wake);
  assert.deepEqual(calls, ['fill', 'outline'], 'a missing country flag has a neutral body rather than only a border');
  assert.equal(canvasCountryFlag(missing, wake), missing);
  const generic: TooltipContent = { title: 'Authored equipment', icon: 'synthetic-art', rows: [] };
  assert.equal(countryFlagContent(generic, undefined, wake), generic);
  const native = countryFlagContent(generic, missing, wake)!;
  assert.equal(native.icon, generic.icon);
  assert.equal(native.countryFlag?.flagReason, missing.flagReason);
  assert.equal(countryFlagUnavailable(missing.flagReason).text, 'Flag unavailable', 'the reason never reaches the label');
  assert.equal(countryFlagUnavailable(undefined, true).text, 'Flag loading');
});

test('scrolling country tooltips retain all rows and their own dated header without decoding hidden flags', () => {
  const before = globalThis.Image;
  let decoded = 0;
  globalThis.Image = class { complete = false; naturalWidth = 0; src = '';
    constructor() { decoded++; } } as unknown as typeof Image;
  try {
    const rows = Array.from({ length: 5000 }, (_, index) => ({ label: `Native member ${index}`,
      countryFlag: { flag: `synthetic-scroll-country-${index}` } }));
    const content: TooltipContent = { title: 'Dated breakdown', countryFlag: { flagReason: 'Missing at the selected date' },
      rows, scrollable: true, scrollId: 'selected-country' };
    const shown = countryTooltipContent(content, { flag: 'synthetic-current-node' }, () => {}, 400)!;
    assert.equal(shown.rows, rows);
    assert.equal(shown.countryFlag?.flagReason, 'Missing at the selected date');
    assert.equal(shown.scrollId, content.scrollId);
    assert.equal(decoded, 0, 'only the layer mounts scrolling row flags');
  } finally { globalThis.Image = before; }
});

test('a country canvas decode failure replaces its URL and preserves full keyboard content', () => {
  const before = globalThis.Image;
  const created: { complete: boolean; naturalWidth: number; onerror?: () => void }[] = [];
  globalThis.Image = class { complete = false; naturalWidth = 0; src = ''; onerror?: () => void;
    constructor() { created.push(this); } } as unknown as typeof Image;
  try {
    const flag = { flag: 'synthetic-country-decode-failure', flagPending: false };
    const wake = () => {};
    assert.deepEqual(canvasCountryFlag(flag, wake), { flagPending: true, flagReason: undefined });
    created[0].complete = true; created[0].onerror!();
    const expected = countryFlagUnavailable(undefined, false, true);
    const content = countryFlagContent({ title: 'Native country', rows: [{ label: 'Partner', countryFlag: flag }],
      spoken: 'The complete untrimmed battle rows' }, flag, wake)!;
    assert.equal(content.countryFlag?.flag, undefined);
    assert.equal(content.countryFlag?.flagPending, false);
    assert.equal(content.countryFlag?.flagReason, expected.reason);
    assert.equal(content.rows[0].countryFlag?.flagReason, expected.reason);
    assert.equal(content.spoken, 'The complete untrimmed battle rows', 'a missing flag adds nothing to what is read out');
  } finally { globalThis.Image = before; }
});

test('country artwork fits its authored proportions without cropping in Canvas slots', () => {
  const before = globalThis.Image;
  globalThis.Image = class { complete = true; naturalWidth = 8; naturalHeight = 8; src = ''; } as unknown as typeof Image;
  try {
    const calls: number[][] = [];
    const context = { drawImage(_image: unknown, ...coordinates: number[]) { calls.push(coordinates); } } as unknown as CanvasRenderingContext2D;
    const flag = { flag: 'synthetic-square-shield' };
    canvasCountryFlag(flag, () => {});
    drawCountryFlag(context, flag, 10, 20, 18, 12, tokens, () => {});
    drawCountryFlag(context, flag, 10, 20, 12, 18, tokens, () => {});
    assert.deepEqual(calls, [[13, 20, 12, 12], [10, 23, 12, 12]]);
  } finally { globalThis.Image = before; }
});
