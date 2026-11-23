import { existsSync } from 'node:fs';
import { mkdir, unlink } from 'node:fs/promises';
import path from 'node:path';
import { sleep } from './cdp.mjs';
import { assertValidPng } from './map.mjs';
import { Coverage, controlBase, placementKey } from './coverage.mjs';
import { Session, TOOLTIP_DELAY_MS } from './session.mjs';

const OPEN_POPOVER = sel => `document.querySelector(${JSON.stringify(`${sel}:popover-open`)})`;
const CLOSED_POPOVER = sel => `!document.querySelector(${JSON.stringify(`${sel}:popover-open`)})`;
const LABELLED = label => `button[aria-label=${JSON.stringify(label)}]`;
const widgetSel = title => `article.dash-widget[aria-label=${JSON.stringify(title)}]`;

const press = (s, id, label, selector, done, options = {}) =>
  s.step('controls', id, label, () => s.click(selector, options.locate ?? {}), { done, ...options });
const back = control => ({ check: false, control });
const option = index => `.select-menu:popover-open [role=option][data-option-index="${index}"]`;

export const CASES = [
  { id: 'header/previous-next-hour', page: 'Overview', async run(s, page) {
    const was = (await s.timeline()).value;
    await press(s, this.id, 'next hour', LABELLED('Next hour'), `Number(document.querySelector('input.header-timeline').value) === ${was + 1}`);
    await press(s, this.id, 'previous hour', LABELLED('Previous hour'), `Number(document.querySelector('input.header-timeline').value) === ${was}`, back('next hour'));
  } },
  { id: 'header/play-pause', page: 'Overview', async run(s) {
    const was = (await s.timeline()).value;
    await press(s, this.id, 'play', 'button.header-play[aria-label="Play"]', `document.querySelector('button.header-play[aria-label="Pause"]')`);
    await s.until(`${Session.CLOCK_HOUR} > ${was}`, 4000);
    await press(s, this.id, 'pause', 'button.header-play[aria-label="Pause"]', `document.querySelector('button.header-play[aria-label="Play"]')`);
  } },
  { id: 'header/speed-menu', page: 'Overview', async run(s) {
    const was = await s.app.evaluate(`document.querySelector('.header-speed').getAttribute('aria-label')`);
    await press(s, this.id, 'open', '.header-speed', OPEN_POPOVER('.playback-speed-menu'));
    await press(s, this.id, 'choose 4 hours', '.playback-speed-menu:popover-open .playback-speed-presets button',
      `${CLOSED_POPOVER('.playback-speed-menu')} && document.querySelector('.header-speed').getAttribute('aria-label') !== ${JSON.stringify(was)}`, { locate: { index: 2 } });
    await press(s, this.id, 'open again', '.header-speed', OPEN_POPOVER('.playback-speed-menu'), back('open'));
    await press(s, this.id, 'choose 1 hour', '.playback-speed-menu:popover-open .playback-speed-presets button', CLOSED_POPOVER('.playback-speed-menu'), { locate: { index: 1 }, ...back('choose 4 hours') });
    const now = await s.app.evaluate(`document.querySelector('.header-speed').getAttribute('aria-label')`);
    if (now !== was) throw new Error(`speed not restored: ${was} became ${now}`);
  } },
  { id: 'header/mute', page: 'Overview', async run(s) {
    const selector = '.header-playback button[aria-label="Mute map audio"], .header-playback button[aria-label="Unmute map audio"]';
    const pressed = `document.querySelector(${JSON.stringify(selector)})?.getAttribute('aria-pressed')`;
    const was = await s.app.evaluate(pressed);
    if (was == null) throw new Error('no map audio button');
    await press(s, this.id, 'toggle', selector, `${pressed} !== ${JSON.stringify(was)}`);
    await press(s, this.id, 'toggle back', selector, `${pressed} === ${JSON.stringify(was)}`, back('toggle'));
  } },
  { id: 'header/date-picker', page: 'Overview', async run(s) {
    const shown = `document.querySelector('.header-date .date-time-value')?.textContent.trim()`;
    const before = await s.app.evaluate(shown);
    await press(s, this.id, 'open', '.header-date button.date-time-trigger', OPEN_POPOVER('.date-time-popover'));
    const month = `document.querySelector('.date-time-popover:popover-open .calendar-grid')?.getAttribute('aria-label')`;
    const arrow = label => `.date-time-popover:popover-open button[aria-label="${label}"]`;
    if (await s.holds(`document.querySelector(${JSON.stringify(arrow('Previous month'))})?.disabled`)) {
      if (await s.holds(`document.querySelector(${JSON.stringify(arrow('Next month'))})?.disabled`)) throw new Error('the recording spans one month: the calendar has no month to move to');
      const shownMonth = await s.app.evaluate(month);
      await press(s, this.id, 'next month', arrow('Next month'), `${month} !== ${JSON.stringify(shownMonth)}`);
    }
    const shownMonth = await s.app.evaluate(month);
    await press(s, this.id, 'previous month', arrow('Previous month'), `${month} !== ${JSON.stringify(shownMonth)}`);
    await press(s, this.id, 'choose the 15th', '.date-time-popover:popover-open .calendar-day[data-day="15"]', `document.querySelector('.header-date.has-date-draft')`);
    await press(s, this.id, 'go', '.date-time-popover:popover-open .calendar-footer button.button.primary', `!document.querySelector('.header-date.has-date-draft') && ${shown} !== ${JSON.stringify(before)}`, { timeout: 10_000 });
  } },
  { id: 'header/country-picker', page: 'Overview', async run(s) {
    const trigger = '.header-workspace-bar button[aria-label^="Choose country"]';
    const was = await s.app.evaluate(`document.querySelector(${JSON.stringify(trigger)})?.textContent.trim() ?? null`);
    if (was === null) throw new Error('no country picker');
    const listed = `Number(document.querySelector('.country-picker:popover-open [role=option]')?.getAttribute('aria-setsize') ?? 0)`;
    const names = `[...document.querySelectorAll('.country-picker:popover-open [role=option] small')].map(e => e.previousElementSibling?.textContent.trim() ?? '')`;
    const narrowed = (query, all) => `(${listed}) > 0 && (${listed}) < ${all} && ${names}.some(name => name.includes(${JSON.stringify(query)}))`;
    const onlyMatches = async (query, label) => {
      const rows = await s.app.evaluate('__e2e.countryRows()');
      const wanted = query.toLowerCase();
      if (!rows || rows.rows.length !== rows.size) await s.fail('controls', this.id, label, `its results could not be read whole (${rows?.rows.length ?? 0} of ${rows?.size ?? '?'} rows)`);
      else {
        const stray = rows.rows.filter(row => !row.name.toLowerCase().includes(wanted) && !row.tag.toLowerCase().includes(wanted));
        if (stray.length) await s.fail('controls', this.id, label, `it also lists ${stray.length} countries not matching "${query}" (${stray.slice(0, 3).map(row => row.name || row.tag).join(', ')})`);
      }
    };
    const shownName = `document.querySelector(${JSON.stringify(trigger)})?.textContent.trim()`;
    await press(s, this.id, 'open', trigger, OPEN_POPOVER('.country-picker'));
    const all = await s.app.evaluate(listed);
    const chosen = (await s.app.evaluate('__e2e.countryRows()'))?.rows.find(row => row.selected) ?? null;
    if (!chosen) throw new Error(`the picker marks no row as the country shown (${was})`);
    const { name, tag } = chosen;
    await s.step('controls', this.id, 'search', () => s.type('Italy'), { done: narrowed('Italy', all) });
    await onlyMatches('Italy', 'search');
    await press(s, this.id, 'choose', '.country-picker:popover-open [role=option] span:has(+ small)', `${CLOSED_POPOVER('.country-picker')} && ${shownName}.includes('Italy')`,
      { timeout: 8000, locate: { text: 'Italy' } });
    await press(s, this.id, 'open again', trigger, OPEN_POPOVER('.country-picker'), back('open'));
    await s.step('controls', this.id, 'search back', () => s.type(name), { done: narrowed(name, all), ...back('search') });
    await onlyMatches(name, 'search back');
    await s.step('controls', this.id, 'choose back', async () => {
      if (!await s.app.evaluate(`__e2e.revealCountry(${JSON.stringify(tag)})`)) throw new Error(`no row for ${tag} in its results`);
      await s.click('.country-picker:popover-open [role=option] small', { text: tag });
    }, { done: `${CLOSED_POPOVER('.country-picker')} && ${shownName} === ${JSON.stringify(was)}`, timeout: 8000, ...back('choose') });
    await s.click(trigger);
    await s.until(`document.querySelector('.country-picker:popover-open [role=option]')`, 3000);
    const back_ = (await s.app.evaluate('__e2e.countryRows()'))?.rows.find(row => row.selected) ?? null;
    if (back_?.tag !== tag) await s.fail('controls', this.id, 'choose back', `the picker marks ${back_?.tag ?? 'no country'} chosen, not ${tag}`);
    await s.escape();
  } },
  { id: 'header/saved-layout', page: 'Overview', async run(s) {
    await press(s, this.id, 'open', '.header-layout-trigger', OPEN_POPOVER('.header-layout-menu'));
    await s.step('controls', this.id, 'close', () => s.escape(), { done: CLOSED_POPOVER('.header-layout-menu') });
  } },
  { id: 'header/edit-catalogue-cancel', page: 'Overview', async run(s) {
    await press(s, this.id, 'edit', LABELLED('Edit layout'), `document.querySelector('button[aria-label="Done editing"]')`);
    await press(s, this.id, 'open catalogue', LABELLED('Add widget'), OPEN_POPOVER('.widget-catalogue'));
    const cards = `document.querySelectorAll('.widget-catalogue:popover-open .widget-card').length`;
    const every = await s.app.evaluate(cards);
    await s.click('.widget-catalogue:popover-open input[type=search]');
    await s.step('controls', this.id, 'search catalogue', () => s.type('map'),
      { done: `(${cards}) > 0 && (${cards}) < ${every} && [...document.querySelectorAll('.widget-catalogue:popover-open .widget-card')].some(c => /map/i.test(c.textContent))` });
    await press(s, this.id, 'close catalogue', LABELLED('Close widget catalogue'), CLOSED_POPOVER('.widget-catalogue'));
    await press(s, this.id, 'cancel', LABELLED('Discard layout changes'), `document.querySelector('button[aria-label="Edit layout"]')`);
  } },
  { id: 'timeline/drag', page: 'Overview', async run(s) {
    const line = await s.timeline();
    if (!line) throw new Error('no timeline');
    const at = f => ({ x: Session.timelineX(line, f), y: line.y });
    const near = f => `Math.abs(Number(document.querySelector('input.header-timeline').value) - ${line.min + f * (line.max - line.min)}) <= ${2 * (line.max - line.min) / (line.width - 14) + 1}`;
    await s.step('controls', this.id, 'drag 30% to 70%', () => s.drag(at(0.3), at(0.7), 40), { timeout: 8000, from: 'last', done: near(0.7) });
    await s.step('controls', this.id, 'drag 70% to 20%', () => s.drag(at(0.7), at(0.2), 40), { timeout: 8000, from: 'last', done: near(0.2) });
    for (let i = 1; i <= 3; i++) {
      const was = (await s.timeline()).value;
      await s.step('controls', this.id, `arrow right ${i}`, () => s.key('ArrowRight'), { timeout: 4000, done: `Number(document.querySelector('input.header-timeline').value) > ${was}`, control: 'arrow right' });
    }
  } },
  { id: 'layout/drag-resize', page: 'Overview', async run(s) {
    await press(s, this.id, 'edit', LABELLED('Edit layout'), `document.querySelector('button[aria-label="Done editing"]')`);
    await s.app.evaluate(`document.querySelector('.dash-grid-scroll').scrollTo({ top: 0 })`);
    await s.frames(2);
    const grip = await s.need('.dash-drag-grip', { index: 1 });
    const box = (key, test) => `(r => ${test})(document.querySelector(${JSON.stringify(`article.dash-widget[data-widget="${key}"]`)})?.getBoundingClientRect())`;
    const moved = await s.app.evaluate(`(() => { const e = [...document.querySelectorAll('.dash-drag-grip')].filter(e => e.getClientRects().length)[1]?.closest('.dash-grid-cell')?.querySelector('article.dash-widget');
      const r = e?.getBoundingClientRect(); return e && { key: e.dataset.widget, left: r.left, top: r.top }; })()`);
    if (!moved) throw new Error('the dragged grip belongs to no widget');
    await s.step('controls', this.id, 'drag widget', () => s.drag(grip, { x: grip.x - 300, y: grip.y + 160 }), { timeout: 6000, from: 'last',
      done: box(moved.key, `r && (Math.abs(r.left - ${moved.left}) > 4 || Math.abs(r.top - ${moved.top}) > 4)`) });
    const handle = await s.need('.react-resizable-handle-se');
    const resized = await s.app.evaluate(`(() => { const e = [...document.querySelectorAll('.react-resizable-handle-se')].filter(e => e.getClientRects().length)[0]?.closest('.dash-grid-cell')?.querySelector('article.dash-widget');
      const r = e?.getBoundingClientRect(); return e && { key: e.dataset.widget, width: r.width, height: r.height }; })()`);
    if (!resized) throw new Error('the resize handle belongs to no widget');
    const sized = test => box(resized.key, `r && ${test}`);
    const handleOf = `.dash-grid-cell:has(article.dash-widget[data-widget=${JSON.stringify(resized.key)}]) .react-resizable-handle-se`;
    await s.step('controls', this.id, 'resize bigger', () => s.drag({ x: handle.x - 2, y: handle.y - 2 }, { x: handle.x + 180, y: handle.y + 140 }), { timeout: 6000, from: 'last',
      done: sized(`r.width > ${resized.width + 4} && r.height > ${resized.height + 4}`) });
    const grown = await s.need(handleOf);
    await s.step('controls', this.id, 'resize smaller', () => s.drag({ x: grown.x - 2, y: grown.y - 2 }, { x: handle.x - 2, y: handle.y - 2 }), { timeout: 6000, from: 'last',
      done: sized(`Math.abs(r.width - ${resized.width}) <= 4 && Math.abs(r.height - ${resized.height}) <= 4`), ...back('resize bigger') });
    await press(s, this.id, 'open save', LABELLED('Save layout'), OPEN_POPOVER('.layout-save-menu'));
    const before = await s.app.evaluate(`document.querySelectorAll('.layout-save-menu button').length`);
    if (!before) throw new Error('no save options');
    const name = `Acceptance ${Date.now()}`;
    await s.click('.layout-save-menu:popover-open input[aria-label="Layout name"]');
    await s.app.evaluate(`document.querySelector('.layout-save-menu:popover-open input[aria-label="Layout name"]').select()`);
    await s.type(name);
    await press(s, this.id, 'save as new', '.layout-save-menu:popover-open .layout-save-actions button.primary',
      `document.querySelector('button[aria-label="Edit layout"]') && document.querySelector('.header-layout-trigger .header-layout-name')?.textContent.trim() === ${JSON.stringify(name)}`);
    for (const [label, index] of [['switch to first layout', 0], ['switch to saved layout', -1]]) {
      await press(s, this.id, `${label}: open`, '.header-layout-trigger', OPEN_POPOVER('.header-layout-menu'));
      const options = await s.app.evaluate(`document.querySelectorAll('.header-layout-menu:popover-open .header-layout-list [role=menuitemradio]').length`);
      if (options < 2) throw new Error(`saving as new left ${options} layouts`);
      const at = index < 0 ? options : index + 1;
      await press(s, this.id, label, `.header-layout-menu:popover-open .header-layout-list [role=menuitemradio]:nth-child(${at})`, CLOSED_POPOVER('.header-layout-menu'), { timeout: 8000 });
    }
  } },
  { id: 'widget/settings', page: '*', run: settingsSweep },
  { id: 'widget/live-toggle', page: '*', async run(s, page) {
    const legacy = await s.app.evaluate(`[...document.querySelectorAll('article.dash-widget')].filter(a => a.querySelector('.live-toggle'))
      .map(a => ({ key: a.dataset.widget, title: a.getAttribute('aria-label'), live: a.querySelector('.live-toggle').getAttribute('aria-checked') }))`);
    for (const { key, title, live } of legacy) {
      const selector = `article.dash-widget[data-widget=${JSON.stringify(key)}] .live-toggle`;
      const is = value => `document.querySelector(${JSON.stringify(selector)})?.getAttribute('aria-checked') === ${JSON.stringify(value)}`;
      const other = live === 'true' ? 'false' : 'true';
      await press(s, this.id, `${page}: ${title} ${live === 'true' ? 'off' : 'on'}`, selector, is(other), { widget: key, control: `${page}: ${title}` });
      await press(s, this.id, `${page}: ${title} ${live === 'true' ? 'on' : 'off'}`, selector, is(live), { widget: key, ...back(`${page}: ${title}`) });
    }
    const chips = await s.app.evaluate(`[...document.querySelectorAll('article.dash-widget')].filter(a => a.querySelector('button.dash-time-chip'))
      .map(a => ({ key: a.dataset.widget, title: a.getAttribute('aria-label') }))`);
    for (const { key, title } of chips) {
      const selector = `article.dash-widget[data-widget=${JSON.stringify(key)}] button.dash-time-chip`;
      await press(s, this.id, `${page}: ${title} time menu`, selector, OPEN_POPOVER('.widget-time-menu'), { widget: key, control: `${page}: ${title}` });
      await s.escape();
      await s.until(CLOSED_POPOVER('.widget-time-menu'), 3000);
    }
  } },
  { id: 'tooltips/chart', page: '*', run: chartTooltips },
  { id: 'tooltips/app', page: 'Overview', async run(s) {
    const at = await s.need(LABELLED('Next hour'));
    await s.hover(5, 5);
    await s.until(`!document.querySelector('.app-tooltip:popover-open')`, 1000);
    await s.frames(2);
    await s.step('controls', this.id, 'header button', () => s.hover(at.x, at.y), { done: OPEN_POPOVER('.app-tooltip'),
      budget: TOOLTIP_DELAY_MS + 100, timeout: 3000 });
    await s.hover(5, 5);
  } },
];

const SETTINGS_MENU = '.widget-settings-menu:popover-open, .map-controls-menu:popover-open';
const IN_MENU = selector => SETTINGS_MENU.split(', ').map(menu => `${menu} ${selector}`).join(', ');
const CONTROL = {
  switch: IN_MENU('input[role=switch]'),
  select: IN_MENU('.select-trigger'),
  number: IN_MENU('.option-number input[type=number]'),
  title: IN_MENU('label.field input:not([type])'),
  'content-set': IN_MENU('.content-set-picker > button[aria-haspopup=dialog]'),
  range: IN_MENU('input[type=range]'),
  check: IN_MENU('input[type=checkbox]:not([role=switch])'),
};
const SAVING = `document.querySelector('.map-appearance[data-saving]')`;
const RESET_CONFIRM = IN_MENU('.map-appearance .graphics-reset-confirm .is-danger');
const MODS_INHERITED = `!document.querySelector(${JSON.stringify(IN_MENU('.map-mod-list[data-overridden]'))})`;
const GRAPHICS_INHERITED = `!document.querySelector(${JSON.stringify(IN_MENU('.map-appearance .map-option-row[data-overridden]'))})`;
const confirmReset = async (s, inherited) => {
  const asking = `document.querySelector(${JSON.stringify(RESET_CONFIRM)})`;
  if (await s.until(`${asking} || (${inherited})`, 2000) && await s.holds(asking)) await s.click(RESET_CONFIRM);
};
const MAP_EXPECTED = `[...document.querySelectorAll(${JSON.stringify(IN_MENU('[data-map-setting]'))})]
  .map(e => [e.getAttribute('data-map-setting'), e.getAttribute('data-map-value')]).filter(([setting, value]) => value !== 'null' || setting.startsWith('graphics.'))`;
const shown = (selector, index) => `[...document.querySelectorAll(${JSON.stringify(selector)})].filter(e => e.getClientRects().length)[${index}]`;

const discover = s => s.app.evaluate(`(() => {
  const menu = document.querySelector(${JSON.stringify(SETTINGS_MENU)}); if (!menu) return [];
  const text = e => e && [...e.childNodes].filter(n => !n.classList?.contains('sr-only')).map(n => n.textContent).join('');
  const out = [], label = e => (text(e.closest('label, .setting-row')?.querySelector('.setting-label, span')) || e.getAttribute('aria-label') || e.textContent || '').trim().slice(0, 60);
  const all = selector => [...menu.querySelectorAll(selector)].filter(e => e.getClientRects().length);
  const draft = e => !!e.closest('.map-appearance');
  const context = e => e.closest('[data-setting]')?.getAttribute('data-setting') ?? '';
  const overridden = e => !!e.closest('.map-option-row, .map-mod-list')?.hasAttribute('data-overridden');
  all('input[role=switch]').forEach((e, i) => out.push({ kind: 'switch', index: i, label: label(e), context: context(e), draft: draft(e), overridden: overridden(e), disabled: e.disabled, state: e.checked,
    mod: !!e.closest('.map-mod-row') }));
  all('input[type=range]').forEach((e, i) => out.push({ kind: 'range', index: i, label: e.getAttribute('aria-label') || label(e), context: context(e), draft: draft(e), overridden: overridden(e),
    disabled: e.disabled, value: Number(e.value), max: Number(e.max), state: Number(e.value) }));
  all('.segmented').forEach((g, i) => { const b = [...g.querySelectorAll('button')]; const on = b.findIndex(x => x.getAttribute('aria-pressed') === 'true');
    out.push({ kind: 'segment', group: i, label: g.getAttribute('aria-label') || 'segment ' + i, context: context(g), draft: draft(g), on, state: on,
      buttons: b.map((x, j) => ({ index: j, label: x.textContent.trim(), disabled: x.disabled })) }); });
  all('.select-trigger').forEach((e, i) => out.push({ kind: 'select', index: i, label: label(e), context: context(e), draft: draft(e), overridden: overridden(e), disabled: e.disabled, state: e.textContent.trim() }));
  all('.option-number input[type=number]').forEach((e, i) => out.push({ kind: 'number', index: i, label: label(e), context: context(e), value: e.value, state: e.value }));
  all('label.field input:not([type])').forEach((e, i) => out.push({ kind: 'title', index: i, label: label(e), context: context(e), state: e.value }));
  all('.content-set-picker > button[aria-haspopup=dialog]').forEach((e, i) => out.push({ kind: 'content-set', index: i, label: e.getAttribute('aria-label') }));
  all('.chip-set').forEach((g, i) => out.push({ kind: 'chips', group: i, label: g.getAttribute('aria-label') || 'chips ' + i, context: context(g), draft: draft(g),
    state: [...g.querySelectorAll('button.chip')].map(x => x.getAttribute('aria-pressed') === 'true'),
    chips: [...g.querySelectorAll('button.chip')].map((x, j) => ({ index: j, label: x.textContent.trim(), pressed: x.getAttribute('aria-pressed') === 'true',
      disabled: x.getAttribute('aria-disabled') === 'true', reason: x.getAttribute('data-tooltip') })) }));
  if (menu.querySelector('[aria-label="Widget size"] button[aria-label=Larger]:not([disabled])')) out.push({ kind: 'size', label: 'size' });
  const fieldLabel = e => (e.closest('.date-time-field')?.querySelector(':scope > span')?.textContent ?? '').trim();
  const special = [
    ['check', 'input[type=checkbox]:not([role=switch])', e => label(e)],
    ['reset', '.map-appearance button.map-menu-action', e => e.textContent.trim()],
    ['search', 'input.map-mod-search', e => e.getAttribute('aria-label')],
    ['refresh', '.map-mod-tools > button', e => e.textContent.trim()],
    ['export', 'button.map-menu-action:not(.map-appearance *)', e => e.textContent.trim()],
    ['country', 'button.widget-country-trigger', e => { const named = e.getAttribute('aria-label') ?? '';
      return named.startsWith('Widget campaign:') ? 'Widget campaign' : named.startsWith('Up to ') || / add or remove$/.test(named) ? 'Add or remove a country' : named; }],
    ['pin', '.setting-row button.button.small', e => e.textContent.trim()],
    ['date', 'button.date-time-trigger', fieldLabel],
    ['colour', 'input[type=color]', e => 'Colour of faction ' + (e.closest('li[data-faction]')?.getAttribute('data-faction') ?? '?')],
    ['faction-name', 'input.faction-name', e => 'Display name of faction ' + (e.closest('li[data-faction]')?.getAttribute('data-faction') ?? '?')],
    ['icon', '[aria-label="Header icon"] button', e => e.getAttribute('aria-label')],
  ];
  const claimed = new Set();
  for (const [kind, selector, name] of special) all(selector).filter(e => kind !== 'pin' || /^Pin to now$/.test(e.textContent.trim()))
    .filter(e => kind !== 'export' || /^Export map image$/.test(e.textContent.trim()))
    .forEach((e, i) => claimed.add(e) && out.push({ kind, index: i, label: (name(e) || kind).slice(0, 80), context: context(e), draft: draft(e), overridden: overridden(e), disabled: e.disabled,
      ...(kind === 'check' ? { state: e.checked } : {}), ...(kind === 'country' ? { trigger: e.getAttribute('aria-label') } : {}) }));
  const handled = 'input[role=switch], input[type=range], .segmented button, .select-trigger, .option-number input[type=number], label.field input:not([type]), '
    + '.content-set-picker > button[aria-haspopup=dialog], .chip-set button.chip, [aria-label="Widget size"] button';
  const frame = '.ui-popover-heading, summary, .map-option-inherit, .select-menu, .content-picker, .country-picker, .campaign-picker, .date-time-popover';
  all('button, input, select, textarea, [role=switch], [tabindex="0"]').filter(e => !e.matches(handled) && !claimed.has(e) && !e.closest(frame) && !e.matches('.folder-info'))
    .forEach(e => out.push({ kind: 'unsupported', label: label(e), what: e.tagName.toLowerCase() + (typeof e.className === 'string' && e.className ? '.' + e.className.split(' ')[0] : '') }));
  const seen = new Map();
  for (const c of out) { const name = c.kind + '|' + c.label + '|' + (c.context ?? ''); c.nth = seen.get(name) ?? 0; seen.set(name, c.nth + 1); }
  return out;
})()`);
const identity = c => `${c.kind}|${c.label}|${c.context ?? ''}|${c.nth}|${(c.buttons ?? c.chips)?.map(b => b.label).join('/') ?? ''}`;
function stateOf(c) {
  const at = selector => shown(IN_MENU(selector), c.kind === 'segment' || c.kind === 'chips' ? c.group : c.index);
  switch (c.kind) {
    case 'switch': return `${at('input[role=switch]')}?.checked`;
    case 'range': return `(e => e && Number(e.value))(${at('input[type=range]')})`;
    case 'segment': return `(g => g ? [...g.querySelectorAll('button')].findIndex(x => x.getAttribute('aria-pressed') === 'true') : undefined)(${at('.segmented')})`;
    case 'select': return `${at('.select-trigger')}?.textContent.trim()`;
    case 'number': return `${at('.option-number input[type=number]')}?.value`;
    case 'title': return `${at('label.field input:not([type])')}?.value`;
    case 'chips': return `(g => g ? [...g.querySelectorAll('button.chip')].map(x => x.getAttribute('aria-pressed') === 'true') : undefined)(${at('.chip-set')})`;
    case 'check': return `${at(CONTROL.check)}?.checked`;
    default: return 'undefined';
  }
}

const QUICK_KINDS = ['switch', 'segment', 'select', 'check', 'chips'];

async function settingsSweep(s, page) {
  const widgets = await s.app.evaluate(`[...document.querySelectorAll('article.dash-widget')].map(a => { const r = a.getBoundingClientRect();
    return { key: a.dataset.widget, title: a.getAttribute('aria-label'), view: a.classList.contains('kind-map') ? a.dataset.widget : null,
      kind: [...a.classList].find(c => c.startsWith('kind-'))?.slice(5) ?? null, width: r.width, height: r.height }; })`);
  s.placements ??= new Map();
  s.quickKinds ??= new Set();
  const quick = s.tier === 'quick';
  for (const widget of widgets) {
    if (quick && s.quickKinds.has(widget.kind)) continue;
    const placement = `${page}: ${widget.title}`;
    await inView(s, widget);
    const opened = await press(s, 'widget/settings', `${placement}: open`, triggerOf(widget), `document.querySelector(${JSON.stringify(SETTINGS_MENU)})`, { control: `${placement}: open` });
    if (!opened.pass && opened.reasons.some(r => r.startsWith('could not act') || r.startsWith('never finished'))) continue;
    const graphics = await reopen(s, widget);
    let controls = await discover(s);
    if (graphics !== 'none' && graphics !== 'ready') {
      await s.fail('controls', 'widget/settings', `${placement}: graphics`, graphics === 'loading' ? 'its graphics settings never loaded within 8 s' : `its graphics settings ${graphics}`);
      controls = controls.filter(c => !c.draft);
    }
    const key = placementKey({ ...widget, controls, neighbours: widgets.filter(other => other !== widget).map(other => `${other.kind}:${Math.round(other.width)}x${Math.round(other.height)}`) });
    const same = s.placements.get(key);
    if (same && !quick && !s.everFailed('controls', 'widget/settings', `${placement}: `)) {
      await s.record({ area: 'controls', item: 'widget/settings', label: `${placement}: the same case as ${same} (kind, size, settings and neighbours): not exercised again`,
        control: `${placement}: open`, ms: null, pass: true, reasons: [], screenshot: false });
      await s.escape();
      continue;
    }
    if (!same) s.placements.set(key, placement);
    if (quick) {
      s.quickKinds.add(widget.kind);
      controls = [...controls.filter(c => QUICK_KINDS.includes(c.kind) && !c.disabled), ...controls.filter(c => !QUICK_KINDS.includes(c.kind) || c.disabled)];
    }
    const ctx = { known: new Set(controls.map(identity)), cover: new Coverage({ limit: quick ? 1 : Infinity }) };
    const before = controls.filter(c => c.state !== undefined && !c.disabled && !(c.kind === 'segment' && c.state < 0)).map(c => ({ ...c }));
    for (const c of controls) {
      if (ctx.cover.full) break;
      await exercise(s, page, widget, c, ctx, null);
      if (!quick) await reconcile(s, page, widget, before, c);
    }
    if (ctx.cover.skipped && !quick) await s.record({ area: 'controls', item: 'widget/settings', label: `${placement}: ${ctx.cover.skipped} values already exercised under another setting of its menu, not repeated`,
      control: `${placement}: open`, ms: null, pass: true, reasons: [], screenshot: false });
    await s.escape();
  }
}

async function reconcile(s, page, widget, before, exercised) {
  const fixed = [];
  for (let pass = 0; pass <= before.length; pass++) {
    await reopen(s, widget);
    const now = new Map((await discover(s)).map(c => [identity(c), c]));
    const wrong = before.filter(c => JSON.stringify(now.get(identity(c))?.state) !== JSON.stringify(c.state));
    if (!wrong.length) break;
    const target = wrong.find(c => now.has(identity(c)));
    if (!target) {
      await s.fail('controls', 'widget/settings', `${page}: ${widget.title}: settings`, `after ${exercised.label}, ${wrong.map(c => c.label).join(', ')} never came back to the menu`);
      return;
    }
    if (identity(target) === identity(exercised))
      await s.fail('controls', 'widget/settings', `${page}: ${widget.title}: ${exercised.label}: back`, `it did not come back to what it was (${JSON.stringify(target.state)}, now ${JSON.stringify(now.get(identity(target)).state)})`);
    try { await putBack(s, target, now.get(identity(target))); } catch (error) {
      await s.fail('controls', 'widget/settings', `${page}: ${widget.title}: settings`, `after ${exercised.label}, ${target.label} could not be set back: ${error.message}`);
      return;
    }
    fixed.push(target.label);
    if (pass === before.length) await s.fail('controls', 'widget/settings', `${page}: ${widget.title}: settings`, `after ${exercised.label}, the menu never settled back to what it was`);
  }
  if (fixed.length) await s.record({ area: 'controls', item: 'widget/settings', label: `${page}: ${widget.title}: ${exercised.label}: dependants set back (${fixed.join(', ')})`,
    control: `${page}: ${widget.title}: ${exercised.label}`, ms: null, pass: true, reasons: [], screenshot: false });
}

async function putBack(s, target, current) {
  const centre = expression => s.app.evaluate(`(() => { const e = ${expression}; if (!e) return null; e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
  const press = async expression => { const at = await centre(expression); if (!at) throw new Error('it is not on screen'); await s.app.click(at.x, at.y); await s.frames(2); };
  if (target.kind === 'switch' || target.kind === 'check') await press(shown(CONTROL[target.kind], current.index));
  else if (target.kind === 'segment') await press(`(${shown(IN_MENU('.segmented'), current.group)}?.querySelectorAll('button') ?? [])[${target.state}]`);
  else if (target.kind === 'chips') {
    for (const [index, pressed] of target.state.entries())
      if (current.state[index] !== pressed) await press(`(${shown(IN_MENU('.chip-set'), current.group)}?.querySelectorAll('button.chip') ?? [])[${index}]`);
  } else if (target.kind === 'select') {
    await press(shown(CONTROL.select, current.index));
    await s.until(OPEN_POPOVER('.select-menu'), 2000);
    const listed = await s.app.evaluate('__e2e.selectOptions()');
    const entry = listed?.options.find(option => option.label === target.state);
    if (!entry || !await s.app.evaluate(`__e2e.revealOption(${entry.index})`)) { await s.escape(); throw new Error(`its option ${target.state} is not in its list`); }
    await s.click(option(entry.index));
  } else if (target.kind === 'number' || target.kind === 'title') {
    const field = shown(CONTROL[target.kind], current.index);
    await press(field);
    await s.app.evaluate(`${field}.select()`);
    if (target.state === '') await s.key('Backspace'); else await s.type(target.state);
    await s.key('Enter');
  } else if (target.kind === 'range') {
    await s.app.evaluate(`${shown(CONTROL.range, current.index)}.focus()`);
    const key = target.state > current.state ? 'ArrowRight' : 'ArrowLeft';
    for (let step = 0; step < Math.abs(target.state - current.state); step++) await s.key(key);
  }
  await s.frames(2);
  if (target.draft) await s.until(`!${SAVING}`, 8000);
}

const widgetAt = key => `article.dash-widget[data-widget=${JSON.stringify(key)}]`;
const inView = (s, widget) => s.app.evaluate(`(() => { const a = document.querySelector(${JSON.stringify(widgetAt(widget.key))}); const grid = document.querySelector('.dash-grid-scroll');
  if (!a || !grid) return; const r = a.getBoundingClientRect(), g = grid.getBoundingClientRect();
  if (r.top < g.top || r.bottom > g.bottom) grid.scrollBy({ top: r.top - g.top, behavior: 'instant' }); })()`).then(() => s.frames(2));
const triggerOf = widget => `${widgetAt(widget.key)} [data-widget-settings], ${widgetAt(widget.key)} button[aria-label="Map settings"]`;
async function openSections(s) {
  for (let open = 0; open < 50; open++) {
    const summary = await s.locate(IN_MENU('details:not([open]) > summary'));
    if (!summary) return;
    await s.app.click(summary.x, summary.y);
    await s.frames(2);
  }
}
const graphicsState = `(() => { const section = document.querySelector('.map-controls-menu:popover-open .map-appearance');
  if (!section) return 'none'; const alert = section.querySelector('[role=alert]');
  return alert ? 'failed: ' + alert.textContent.trim() : section.querySelector('.graphics-inheritance') ? 'ready' : 'loading'; })()`;
const reopen = async (s, widget) => {
  if (!await s.holds(`document.querySelector(${JSON.stringify(SETTINGS_MENU)})`)) {
    await inView(s, widget);
    await s.click(triggerOf(widget));
    await s.until(`document.querySelector(${JSON.stringify(SETTINGS_MENU)})`, 2000);
  }
  await openSections(s);
  if (await s.app.evaluate(graphicsState) === 'loading') await s.until(`${graphicsState} !== 'loading'`, 8000);
  return s.app.evaluate(graphicsState);
};

async function exercise(s, page, widget, c, ctx, under) {
  try { await exerciseOnce(s, page, widget, c, ctx, under); } catch (error) {
    if (!error.gone) throw error;
    await s.fail('controls', 'widget/settings', `${page}: ${widget.title}: ${c.label}${under ? ` (with ${under})` : ''}`, 'the control left its menu after a change elsewhere in it');
  }
}

async function exerciseOnce(s, page, widget, c, ctx, under) {
  const { known, cover } = ctx;
  if (cover.full) return;
  const { key, title } = widget;
  const changes = widget.view ? { key, view: widget.view } : { key };
  const name = `${page}: ${title}: ${c.label}${under ? ` (with ${under})` : ''}`;
  const id = { control: name };
  const take = (value, label = name) => cover.take(controlBase(c), value, s.everFailed('controls', 'widget/settings', label));
  const step = (label, action, options = {}) => s.step('controls', 'widget/settings', label, action, { widget: key, ...id, ...options });
  const set = async (label, action, options = {}) => {
    if (!c.draft && !widget.view) return step(label, action, { changes, timeout: 3000, ...options });
    if (!c.draft) {
      const since = s.app.lastSent('set_dashboard_maps');
      const was = (await discover(s)).find(x => identity(x) === identity(c))?.state;
      const changedState = was === undefined ? null : `(() => { const now = ${stateOf(c)}; return now !== undefined && JSON.stringify(now) !== ${JSON.stringify(JSON.stringify(was))}; })()`;
      const result = await step(label, action, { timeout: 3000, awaits: false, ...options,
        done: options.done && changedState ? `(${options.done}) && ${changedState}` : options.done ?? changedState });
      const now = (await discover(s)).find(x => identity(x) === identity(c))?.state;
      if (was !== undefined && JSON.stringify(now) === JSON.stringify(was)) await s.fail('controls', 'widget/settings', label, 'its control did not change');
      if (await s.holds(`document.querySelector(${JSON.stringify(SETTINGS_MENU)})`))
        await step(`${label}: drawn`, () => s.key('Escape'), { changes: { key, view: widget.view, since }, timeout: 3000, check: options.check, control: `${options.control ?? name}: drawn` });
      else await s.fail('controls', 'widget/settings', `${label}: drawn`, 'its menu closed by itself, so the map could not be timed from its dismissal');
      return result;
    }
    const saved = await step(label, action, { ...options, done: options.done ? `(${options.done}) && !${SAVING}` : `!${SAVING}`, timeout: 8000 });
    if (!saved.pass && saved.reasons.some(reason => reason.startsWith('could not act') || reason.startsWith('never finished'))) return saved;
    if (!widget.view) { await s.fail('controls', 'widget/settings', `${label}: drawn`, 'its menu saves map graphics, but its widget shows no map to draw them'); return saved; }
    await reopen(s, widget);
    const expected = await s.app.evaluate(MAP_EXPECTED);
    await step(`${label}: drawn`, () => s.key('Escape'), { drew: { view: widget.view, expected }, timeout: 20_000, check: options.check, control: `${options.control ?? name}: drawn` });
    return saved;
  };
  const alreadyBack = async label => {
    const now = (await discover(s)).find(x => identity(x) === identity(c));
    if (!now || now.state === undefined || JSON.stringify(now.state) !== JSON.stringify(c.state) || (c.draft && !!now.overridden !== !!c.overridden)) return false;
    await s.record({ area: 'controls', item: 'widget/settings', label: `${label}: already back`, ...id, ms: null, pass: true, reasons: [], screenshot: false });
    return true;
  };
  const restore = async (label, reverse) => {
    if (s.tier === 'quick') return;
    await reopenHere();
    if (await alreadyBack(label)) return;
    if (!c.draft || c.overridden || c.kind === 'select') return reverse();
    return set(label, async () => {
      if (c.mod) {
        await s.click(IN_MENU('.map-mods-inherit'));
        await confirmReset(s, MODS_INHERITED);
        return;
      }
      const at = await s.app.evaluate(`(() => { const e = ${shown(CONTROL[c.kind], c.index)}?.closest('.map-option-row')?.querySelector('.map-option-inherit');
        if (!e) return null; e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
      if (!at) throw new Error('its row offers no way back to inheriting');
      await s.app.click(at.x, at.y);
    }, { check: false, ...(c.mod ? { done: MODS_INHERITED } : {}) });
  };
  const revealed = async change => {
    if (cover.full) return;
    await reopen(s, widget);
    for (const fresh of await discover(s)) {
      if (cover.full) return;
      if (known.has(identity(fresh))) continue;
      known.add(identity(fresh));
      await exercise(s, page, widget, fresh, ctx, change);
      await reopen(s, widget);
    }
  };
  const reopenHere = async () => {
    await reopen(s, widget);
    const now = (await discover(s)).find(x => identity(x) === identity(c));
    if (!now) throw Object.assign(new Error('the control left its menu'), { gone: true });
    Object.assign(c, { index: now.index, group: now.group });
  };
  if (!['segment', 'select', 'chips'].includes(c.kind) && !take('')) return;
  await reopenHere();
  if (c.kind === 'unsupported') { await s.fail('controls', 'widget/settings', name, `not exercised: the sweep has no case for this control (${c.what})`); return; }
  if (c.disabled) { await s.record({ area: 'controls', item: 'widget/settings', label: `${name}: disabled`, ...id, ms: null, pass: true, reasons: [], disabled: true, screenshot: false }); return; }
  if (c.kind === 'switch') {
    await set(name, () => s.click(CONTROL.switch, { index: c.index }));
    await revealed(`${c.label} switched`);
    await reopenHere();
    await restore(`${name} back`, () => set(`${name} back`, () => s.click(CONTROL.switch, { index: c.index }), { check: false }));
  } else if (c.kind === 'segment') {
    const segment = index => async () => {
      const at = await s.app.evaluate(`(() => { const e = (${shown(IN_MENU('.segmented'), c.group)}?.querySelectorAll('button') ?? [])[${index}];
        if (!e) return null; e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
      if (!at) throw new Error('segment gone');
      await s.app.click(at.x, at.y);
    };
    let moved = false;
    for (const button of c.buttons.filter(button => button.index !== c.on && !button.disabled)) {
      if (!take(button.label, `${name} = ${button.label}`)) continue;
      moved = true;
      await reopenHere();
      await set(`${name} = ${button.label}`, segment(button.index));
      await revealed(`${c.label} = ${button.label}`);
    }
    if (moved && s.tier !== 'quick' && c.on >= 0 && c.buttons.length > 1) { await reopenHere(); await set(`${name} back`, segment(c.on), { check: false }); }
  } else if (c.kind === 'select') {
    const open = (label, options = {}) => step(`${name}: ${label}`, () => s.click(CONTROL.select, { index: c.index }), { done: OPEN_POPOVER('.select-menu'), control: `${name}: open`, ...options });
    await open('open');
    const listed = await s.app.evaluate('__e2e.selectOptions()');
    await s.escape();
    if (!listed) { await s.fail('controls', 'widget/settings', name, 'its menu never opened'); return; }
    if (listed.options.length !== listed.size) await s.fail('controls', 'widget/settings', name, `its menu lists ${listed.size} options but showed only ${listed.options.length}`);
    const was = listed.options.find(entry => entry.selected) ?? null;
    const reveal = async entry => {
      if (await s.app.evaluate(`__e2e.revealOption(${entry.index}) && document.querySelector(${JSON.stringify(option(entry.index))})?.textContent.trim() === ${JSON.stringify(entry.label)}`)) return entry.index;
      if (!await s.holds(OPEN_POPOVER('.select-menu'))) await open(`open for ${entry.label} again`, { check: false });
      const now = (await s.app.evaluate('__e2e.selectOptions()'))?.options.find(item => item.label === entry.label && !item.disabled);
      return now && await s.app.evaluate(`__e2e.revealOption(${now.index})`) ? now.index : null;
    };
    let moved = false;
    for (const entry of listed.options.filter(entry => entry !== was && !entry.disabled)) {
      if (!take(entry.label, `${name}: ${entry.label}`)) continue;
      moved = true;
      await reopenHere();
      await open(`open for ${entry.label}`, { check: false });
      const at = await reveal(entry);
      if (at === null) { await s.fail('controls', 'widget/settings', `${name}: ${entry.label}`, 'its option never showed in the list'); await s.escape(); continue; }
      await set(`${name}: ${entry.label}`, () => s.click(option(at)), { done: CLOSED_POPOVER('.select-menu') });
      await revealed(`${c.label} = ${entry.label}`);
    }
    if (moved && was && listed.options.length > 1) {
      await reopenHere();
      await restore(`${name}: back`, async () => {
        await open('open again', { check: false });
        const at = await reveal(was);
        if (at !== null) await set(`${name}: back`, () => s.click(option(at)), { done: CLOSED_POPOVER('.select-menu'), check: false });
        else { await s.fail('controls', 'widget/settings', `${name}: back`, 'its option never showed in the list'); await s.escape(); }
      });
    }
  } else if (c.kind === 'chips') {
    const chipAt = index => `(${shown(IN_MENU('.chip-set'), c.group)}?.querySelectorAll('button.chip') ?? [])[${index}]`;
    const chip = index => async () => {
      const at = await s.app.evaluate(`(() => { const e = ${chipAt(index)}; if (!e) return null; e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
      if (!at) throw new Error('chip gone');
      await s.app.click(at.x, at.y);
    };
    const pressed = (index, value) => `${chipAt(index)}?.getAttribute('aria-pressed') === ${JSON.stringify(String(value))}`;
    for (const entry of c.chips) {
      const chipName = `${name}: ${entry.label}`;
      if (!take(entry.label, chipName)) continue;
      if (entry.disabled) { await s.record({ area: 'controls', item: 'widget/settings', label: `${chipName}: disabled (${entry.reason ?? 'no reason given'})`, control: chipName, ms: null, pass: true, reasons: [], disabled: true, screenshot: false }); continue; }
      await reopenHere();
      await set(chipName, chip(entry.index), { done: pressed(entry.index, !entry.pressed), control: chipName });
      if (s.tier === 'quick') continue;
      await reopenHere();
      await set(`${chipName} back`, chip(entry.index), { done: pressed(entry.index, entry.pressed), check: false, control: chipName });
    }
  } else if (c.kind === 'range') {
    const [forth, backward] = c.value >= c.max ? ['ArrowLeft', 'ArrowRight'] : ['ArrowRight', 'ArrowLeft'];
    const nudge = key => async () => { await s.app.evaluate(`${shown(CONTROL.range, c.index)}.focus()`); await s.key(key); };
    await set(name, nudge(forth));
    await reopenHere();
    await restore(`${name} back`, () => set(`${name} back`, nudge(backward), { check: false }));
  } else if (c.kind === 'number') {
    await s.click(CONTROL.number, { index: c.index });
    await set(`${name}: up`, () => s.key('ArrowUp'));
    await reopenHere();
    const field = shown(CONTROL.number, c.index);
    await set(`${name}: back`, async () => {
      await s.click(CONTROL.number, { index: c.index });
      await s.app.evaluate(`${field}.select()`);
      if (c.value === '') await s.key('Backspace'); else await s.type(c.value);
      await s.key('Enter');
    }, { check: false, from: 'last', done: `${field}?.value === ${JSON.stringify(c.value)}` });
  } else if (c.kind === 'check') {
    await set(name, () => s.click(CONTROL.check, { index: c.index }));
    await revealed(`${c.label} ${c.state ? 'cleared' : 'ticked'}`);
    await reopenHere();
    await restore(`${name} back`, () => set(`${name} back`, () => s.click(CONTROL.check, { index: c.index }), { check: false }));
  } else if (c.kind === 'reset') {
    const button = IN_MENU('.map-appearance button.map-menu-action');
    const inherited = await s.app.evaluate(`!!${shown(button, c.index)}?.classList.contains('map-mods-inherit')`) ? MODS_INHERITED : GRAPHICS_INHERITED;
    await set(name, async () => {
      await s.click(button, { index: c.index });
      await confirmReset(s, inherited);
    }, { done: inherited });
  } else if (c.kind === 'search') {
    const field = IN_MENU('input.map-mod-search');
    const rows = `[...document.querySelectorAll(${JSON.stringify(IN_MENU('.map-mod-row'))})].filter(e => e.getClientRects().length)`;
    const names = `(${rows}).map(e => (e.querySelector('span')?.firstChild?.textContent ?? '').toLowerCase())`;
    const every = await s.app.evaluate(`(${rows}).length`);
    const first = await s.app.evaluate(`(${rows})[0]?.querySelector('span')?.firstChild?.textContent?.trim() ?? ''`);
    const query = (first.split(/\s+/).find(word => word.length >= 4) ?? first).slice(0, 12);
    if (!query) { await s.record({ area: 'controls', item: 'widget/settings', label: `${name}: no mods to search`, ...id, ms: null, pass: true, reasons: [], screenshot: false }); return; }
    const wanted = JSON.stringify(query.toLowerCase());
    await step(`${name}: search`, async () => { await s.click(field); await s.type(query); },
      { done: `(${names}).length > 0 && (${names}).every(text => text.includes(${wanted}))`, from: 'last', awaits: false });
    await step(`${name}: clear`, async () => {
      await s.click(field);
      await s.app.evaluate(`document.querySelector(${JSON.stringify(field)})?.select()`);
      await s.key('Backspace');
    }, { done: `(${rows}).length === ${every}`, from: 'last', awaits: false, check: false, control: `${name}: search` });
  } else if (c.kind === 'refresh') {
    const button = IN_MENU('.map-mod-tools > button');
    await step(name, () => s.click(button), { done: `document.querySelector(${JSON.stringify(button)})?.disabled === false && !document.querySelector(${JSON.stringify(IN_MENU('.map-menu-error'))})`,
      awaits: false, timeout: 8000 });
  } else if (c.kind === 'export') {
    if (!s.native || !s.work) { await s.fail('controls', 'widget/settings', name, "not exercised: the controls phase was given no native input for the host's save dialog"); return; }
    const folder = path.join(s.work, 'control-exports');
    await mkdir(folder, { recursive: true });
    const file = path.join(folder, `${name.replace(/[^\w.-]+/g, '_').slice(0, 80)}.png`);
    if (existsSync(file)) await unlink(file);
    const exportSince = Date.now();
    await step(name, async () => {
      await s.click(IN_MENU('button.map-menu-action'), { text: 'Export map image', partial: true });
      await s.native.ask({ op: 'save_dialog', path: file, seconds: 20 });
    }, { awaits: false, check: false, timeout: 3000 });
    const done = await s.app.waitForIpc('export_map', exportSince, 20_000);
    if (!done) await s.fail('controls', 'widget/settings', `${name}: written`, 'export_map did not finish within 20 s');
    else if (!done.ok) await s.fail('controls', 'widget/settings', `${name}: written`, `export_map failed: ${done.error ?? `status ${done.status}`}`);
    else if (done.result === null) await s.fail('controls', 'widget/settings', `${name}: written`, 'export_map was cancelled before writing an image');
    else {
      try {
        await assertValidPng(file);
        await s.record({ area: 'controls', item: 'widget/settings', label: `${name}: written`, ...id, ms: null, pass: true, reasons: [], screenshot: false });
      } catch (error) {
        await s.fail('controls', 'widget/settings', `${name}: written`, `exported PNG is invalid: ${error.message}`);
      }
    }
  } else if (c.kind === 'country') {
    await exerciseCountry(s, c, name, id, step, set, reopenHere);
  } else if (c.kind === 'pin') {
    const pin = IN_MENU('.setting-row button.button.small');
    const held = `${shown(pin, c.index)}?.closest('.setting-row')?.textContent ?? null`;
    const before = await s.app.evaluate(held);
    const ahead = await moveClock(s, 24);
    if (ahead) { await s.fail('controls', 'widget/settings', name, `not exercised: ${ahead}`); return; }
    await reopenHere();
    await set(name, () => s.click(pin, { index: c.index }), { done: `${held} !== ${JSON.stringify(before)}` });
    const backAgain = await moveClock(s, -24);
    if (backAgain) { await s.fail('controls', 'widget/settings', `${name} back`, `not exercised: ${backAgain}`); return; }
    await reopenHere();
    await set(`${name} back`, () => s.click(pin, { index: c.index }), { done: `${held} === ${JSON.stringify(before)}`, check: false });
  } else if (c.kind === 'date') {
    const trigger = IN_MENU('button.date-time-trigger');
    const popover = '.date-time-popover:popover-open';
    const value = `${shown(trigger, c.index)}?.querySelector('.date-time-value')?.textContent ?? null`;
    const before = await s.app.evaluate(value);
    const open = (label, options = {}) => step(`${name}: ${label}`, () => s.click(trigger, { index: c.index }), { done: OPEN_POPOVER('.date-time-popover'), control: `${name}: open`, ...options });
    await open('open');
    const day = await s.app.evaluate(`(() => { const days = [...document.querySelectorAll('${popover} button.calendar-day')].filter(b => !b.disabled && b.parentElement.getAttribute('aria-selected') !== 'true');
      return (days.find(b => b.dataset.day === '15') ?? days[Math.floor(days.length / 2)])?.dataset.day ?? null; })()`);
    if (!day) { await s.escape(); await s.record({ area: 'controls', item: 'widget/settings', label: `${name}: offers no other day`, ...id, ms: null, pass: true, reasons: [], screenshot: false }); return; }
    await set(`${name}: day ${day}`, () => s.click(`${popover} button.calendar-day[data-day="${day}"]`), { done: `${value} !== ${JSON.stringify(before)}` });
    if (await s.holds(OPEN_POPOVER('.date-time-popover'))) await s.escape();
    await reopenHere();
    await open('open again', { check: false });
    await set(`${name}: back`, () => s.click(`${popover} .calendar-shortcuts button`, { text: c.label === 'To' ? 'Last date' : 'First date' }),
      { done: `${value} === ${JSON.stringify(before)}`, check: false });
    if (await s.holds(OPEN_POPOVER('.date-time-popover'))) await s.escape();
  } else if (c.kind === 'colour') {
    if (!s.native) { await s.fail('controls', 'widget/settings', name, 'not exercised: the controls phase was given no native input for the colour dialog'); return; }
    const input = shown(IN_MENU('input[type=color]'), c.index);
    const colour = `(${input}?.value ?? '').toLowerCase()`;
    const before = await s.app.evaluate(colour);
    const resetOf = `${input}?.closest('li')?.querySelector('button.icon-button')`;
    const hadOwn = await s.holds(resetOf);
    const target = before === '#c03a8a' ? '#3a8ac0' : '#c03a8a';
    const choose = hex => async () => {
      await s.click(IN_MENU('input[type=color]'), { index: c.index });
      await s.native.ask({ op: 'color_dialog', rgb: [1, 3, 5].map(at => parseInt(hex.slice(at, at + 2), 16)), seconds: 10 });
    };
    await set(`${name}: ${target}`, choose(target), { done: `${colour} === ${JSON.stringify(target)}`, timeout: 15_000 });
    await reopenHere();
    const reset = async () => {
      const at = await s.app.evaluate(`(() => { const e = ${resetOf}; if (!e) return null; e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
      if (!at) throw new Error('its row offers no reset');
      await s.app.click(at.x, at.y);
    };
    await set(`${name}: back`, !hadOwn ? reset : choose(before), { done: `${colour} === ${JSON.stringify(before)}`, check: false, timeout: 15_000 });
  } else if (c.kind === 'faction-name') {
    const field = shown(IN_MENU('input.faction-name'), c.index);
    const before = await s.app.evaluate(`${field}?.value ?? ''`);
    const placeholder = await s.app.evaluate(`${field}?.placeholder ?? ''`);
    const type = text => async () => {
      await s.click(IN_MENU('input.faction-name'), { index: c.index });
      await s.app.evaluate(`${field}.select()`);
      if (text) await s.type(text); else await s.key('Backspace');
      await s.key('Tab');
    };
    await set(`${name}: rename`, type(`${placeholder || 'Faction'} (renamed)`), { from: 'last' });
    await reopenHere();
    await set(`${name}: rename back`, type(before), { from: 'last', check: false, done: `(${field}?.value ?? '') === ${JSON.stringify(before)}` });
  } else if (c.kind === 'icon') {
    await exerciseIcon(s, c, name, id, step, reopenHere, key);
  } else if (c.kind === 'size') {
    const size = which => IN_MENU(`[aria-label="Widget size"] button[aria-label=${which}]`);
    await set(`${name}: larger`, () => s.click(size('Larger')));
    await reopenHere();
    await set(`${name}: smaller`, () => s.click(size('Smaller')), { check: false });
  } else if (c.kind === 'title') {
    const rename = async (to, label, options = {}) => {
      await reopenHere();
      return step(label, async () => {
        await s.click(CONTROL.title, { index: c.index });
        await s.app.evaluate(`${shown(CONTROL.title, c.index)}.select()`);
        await s.type(to);
        await s.key('Enter');
      }, { done: `document.querySelector(${JSON.stringify(widgetAt(key))})?.getAttribute('aria-label') === ${JSON.stringify(to)}`, from: 'last', ...options });
    };
    const result = await rename(`${title} (renamed)`, `${name}: rename`);
    await s.escape();
    if (result.reasons.some(reason => reason.startsWith('could not act'))) return;
    await rename(title, `${name}: rename back`, { check: false });
    await s.escape();
  } else if (c.kind === 'content-set') {
    const picker = '.content-picker:popover-open';
    const isOpen = `document.querySelector('${picker}')`;
    const openPicker = (label, options = {}) => step(`${name}: ${label}`, () => s.click(CONTROL['content-set'], { index: c.index }),
      { done: `document.querySelector('${picker} [role=option], ${picker} p.content-picker-empty:not([role])')`, control: `${name}: open`, ...options });
    const close = () => step(`${name}: close`, () => s.key('Escape'), { done: `!${isOpen}`, control: `${name}: close` });
    const opened = await openPicker('open');
    if (!opened.pass && opened.reasons.some(reason => reason.startsWith('could not act') || reason.startsWith('never finished'))) { await s.escape(); return; }
    const listed = await s.app.evaluate('__e2e.pickerRows()');
    if (!listed) { await s.fail('controls', 'widget/settings', name, 'its list never showed: no rows and no word that it has none'); return; }
    if (listed.rows.length !== listed.size) await s.fail('controls', 'widget/settings', name, `its list holds ${listed.size} rows but showed only ${listed.rows.length}`);
    const automatic = await s.holds(`document.querySelector('${picker} button[aria-pressed="true"]')`);
    const multiple = await s.holds(`document.querySelector('${picker} [role=listbox]')?.getAttribute('aria-multiselectable') === 'true'`);
    const original = listed.rows.filter(row => row.selected).map(row => row.value).sort();
    const choice = listed.rows.find(row => !row.selected && !row.disabled);
    if (!choice) {
      await close();
      await s.record({ area: 'controls', item: 'widget/settings', label: `${name}: offers no choice`, ...id, ms: null, pass: true, reasons: [], screenshot: false });
      return;
    }
    const pick = value => async () => {
      if (!await s.app.evaluate(`__e2e.revealPickerRow(${JSON.stringify(value)})`)) throw new Error('its row never showed in the list');
      await s.click(`${picker} [role=option][data-value=${JSON.stringify(value)}]`);
    };
    const ensureOpen = async () => {
      await reopenHere();
      if (!await s.holds(isOpen)) { await s.click(CONTROL['content-set'], { index: c.index }); await s.until(`document.querySelector('${picker} [role=option], ${picker} p.content-picker-empty:not([role])')`, 3000); }
    };
    await set(`${name}: ${choice.label}`, pick(choice.value));
    await ensureOpen();
    if (automatic) await set(`${name}: back`, () => s.click(`${picker} button[aria-pressed]`), { check: false });
    else if (multiple) await set(`${name}: back`, pick(choice.value), { check: false });
    else if (original.length) await set(`${name}: back`, pick(original[0]), { check: false });
    else await s.fail('controls', 'widget/settings', `${name}: back`, 'it had neither a choice nor automatic, so there is nothing to put back');
    await ensureOpen();
    const after = await s.app.evaluate('__e2e.pickerRows()');
    if (!after) await s.fail('controls', 'widget/settings', `${name}: back`, 'its list never showed again, so whether its selection came back is unobserved');
    else if (after.rows.length !== after.size) await s.fail('controls', 'widget/settings', `${name}: back`, `its list holds ${after.size} rows but showed only ${after.rows.length}, so whether its selection came back is unobserved`);
    else {
      const now = after.rows.filter(row => row.selected).map(row => row.value).sort();
      if (JSON.stringify(now) !== JSON.stringify(original)) await s.fail('controls', 'widget/settings', `${name}: back`, `its original selection did not come back (${original.length} chosen before, ${now.length} now)`);
    }
    if (await s.holds(isOpen)) await close();
  }
}

const NO_OTHER = (s, id, label) => s.record({ area: 'controls', item: 'widget/settings', label, ...id, ms: null, pass: true, reasons: [], screenshot: false });
const couldNotAct = result => !result.pass && result.reasons.some(reason => reason.startsWith('could not act') || reason.startsWith('never finished'));

async function moveClock(s, hours) {
  await s.escape();
  const was = (await s.timeline())?.value;
  if (was === undefined) return 'there is no timeline';
  const at = `Number(document.querySelector('input.header-timeline')?.value)`;
  for (let step = 1; step <= Math.abs(hours); step++) {
    await s.click(LABELLED(hours > 0 ? 'Next hour' : 'Previous hour'));
    if (!await s.until(`${at} === ${was + Math.sign(hours) * step}`, 2000)) break;
  }
  const now = (await s.timeline()).value;
  return now === was + hours ? null : `the clock stood at ${now}, not ${was + hours}`;
}

async function exerciseCountry(s, c, name, id, step, set, reopenHere) {
  const trigger = IN_MENU('button.widget-country-trigger');
  const opened = picker => `document.querySelector('${picker}')`;
  if (c.label === 'Widget campaign') {
    const picker = '.campaign-picker:popover-open';
    const label = `${shown(trigger, c.index)}?.getAttribute('aria-label') ?? null`;
    const open = (text, options = {}) => step(`${name}: ${text}`, () => s.click(trigger, { index: c.index }), { done: opened(picker), control: `${name}: open`, ...options });
    if (couldNotAct(await open('open'))) { await s.escape(); return; }
    const items = await s.app.evaluate(`[...document.querySelectorAll('${picker} button.menu-item')].filter(b => b.getClientRects().length)
      .map(b => ({ label: b.querySelector('.menu-item-label')?.firstChild?.textContent.trim() ?? '', pressed: b.getAttribute('aria-pressed') === 'true' }))`);
    const was = items.findIndex(item => item.pressed), choice = items.findIndex(item => !item.pressed);
    if (was < 0 || choice < 0) { await s.escape(); await NO_OTHER(s, id, `${name}: offers no other campaign`); return; }
    const before = await s.app.evaluate(label);
    await step(`${name}: ${items[choice].label}`, () => s.click(`${picker} button.menu-item`, { index: choice }), { done: `${label} !== ${JSON.stringify(before)}`, timeout: 8000 });
    await reopenHere();
    if (!await s.holds(opened(picker))) await open('open again', { check: false });
    await step(`${name}: back`, () => s.click(`${picker} button.menu-item`, { index: was }), { done: `${label} === ${JSON.stringify(before)}`, timeout: 8000, check: false });
    return;
  }
  const picker = '.country-picker:popover-open';
  const multi = c.label === 'Add or remove a country';
  const open = (text, options = {}) => step(`${name}: ${text}`, () => s.click(trigger, { index: c.index }),
    { done: `document.querySelector('${picker} [role=option]')`, control: `${name}: open`, timeout: 8000, ...options });
  if (couldNotAct(await open('open'))) { await s.escape(); return; }
  const rows = (await s.app.evaluate('__e2e.countryRows()'))?.rows ?? [];
  const chosen = rows.filter(row => row.selected);
  const full = (c.trigger ?? '').startsWith('Up to ');
  const choice = full ? chosen[0] : rows.find(row => !row.selected && row.tag);
  if (!choice) { await s.escape(); await NO_OTHER(s, id, `${name}: offers no other country`); return; }
  const back = multi ? choice.tag : chosen[0]?.tag;
  const pick = tag => async () => {
    if (!await s.app.evaluate(`__e2e.revealCountry(${JSON.stringify(tag)})`)) throw new Error(`no row for ${tag} in its list`);
    await s.click(`${picker} [role=option] small`, { text: tag });
  };
  await set(`${name}: ${choice.name || choice.tag}`, pick(choice.tag));
  if (!back) { await s.fail('controls', 'widget/settings', `${name}: back`, 'its picker marked no country chosen, so there is none to choose again'); return; }
  await reopenHere();
  if (!await s.holds(opened(picker))) await open('open again', { check: false });
  await set(`${name}: back`, pick(back), { check: false });
  if (await s.holds(opened(picker))) await s.escape();
}

async function exerciseIcon(s, c, name, id, step, reopenHere, key) {
  const header = `document.querySelector(${JSON.stringify(`[data-widget-icon="${key}"]`)})`;
  const look = `(e => e ? e.innerHTML + '|' + (e.getAttribute('style') ?? '') : null)(${header})`;
  const before = await s.app.evaluate(look);
  const button = IN_MENU('[aria-label="Header icon"] button');
  if (c.label === 'Reset icon') {
    await step(name, () => s.click(button, { index: c.index }), { done: `${look} !== ${JSON.stringify(before)}`, awaits: false });
    return;
  }
  const picker = '.icon-style-picker:popover-open';
  const colour = c.label === 'Change icon colour';
  if (couldNotAct(await step(`${name}: open`, () => s.click(button, { index: c.index }), { done: opened(picker), control: `${name}: open`, awaits: false }))) return;
  const resetLabel = colour ? 'Reset icon colour' : 'Reset icon';
  const reset = `${picker} button[aria-label="${resetLabel}"]`;
  const hadOwn = await s.holds(`document.querySelector('${reset}')`);
  const hex = `${picker} input.color-hex`;
  const grid = `${picker} .icon-style-grid button`;
  const typeHex = value => async () => {
    await s.click(hex);
    await s.app.evaluate(`document.querySelector('${hex}').select()`);
    await s.type(value);
    await s.key('Enter');
  };
  let original;
  if (colour) {
    original = await s.app.evaluate(`document.querySelector('${hex}')?.value ?? ''`);
    const target = original.toLowerCase() === '#c03a8a' ? '#3a8ac0' : '#c03a8a';
    await step(`${name}: ${target}`, typeHex(target), { done: `${look} !== ${JSON.stringify(before)}`, from: 'last', awaits: false });
  } else {
    const icons = `[...document.querySelectorAll('${grid}')]`;
    original = await s.app.evaluate(`${icons}.findIndex(b => b.getAttribute('aria-pressed') === 'true')`);
    const other = await s.app.evaluate(`${icons}.findIndex(b => b.getAttribute('aria-pressed') !== 'true')`);
    if (other < 0) { await s.escape(); await NO_OTHER(s, id, `${name}: offers no other icon`); return; }
    await step(`${name}: another icon`, () => s.click(grid, { index: other }), { done: `${look} !== ${JSON.stringify(before)}`, awaits: false });
  }
  const options = { done: `${look} === ${JSON.stringify(before)}`, awaits: false, check: false, control: name };
  if (!hadOwn && await s.holds(`document.querySelector('${reset}')`)) await step(`${name}: back`, () => s.click(reset), options);
  else if (colour) await step(`${name}: back`, typeHex(original), { ...options, from: 'last' });
  else if (original >= 0) await step(`${name}: back`, () => s.click(grid, { index: original }), options);
  else await s.fail('controls', 'widget/settings', `${name}: back`, 'its picker marked no icon chosen, so there is none to choose again');
  if (await s.holds(opened(picker))) await s.escape();
  await reopenHere();
}
const opened = picker => `document.querySelector('${picker}')`;

const CHART_SURFACE = '.dash-widget-content .time-chart canvas';

async function chartTooltips(s, page) {
  const titles = await s.app.evaluate(`[...document.querySelectorAll('article.dash-widget')].map(a => a.getAttribute('aria-label'))`);
  for (const title of titles) {
    const box = await s.app.evaluate(`(() => { const a = document.querySelector(${JSON.stringify(widgetSel(title))}); a?.scrollIntoView({ block: 'center' });
      const c = a?.querySelector(${JSON.stringify(CHART_SURFACE)}); if (!c) return null; const r = c.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
    if (!box) continue;
    await s.hover(5, 5);
    await s.until(`!document.querySelector('.chart-tooltip:popover-open')`, 1000);
    await s.frames(2);
    await s.step('controls', 'tooltips/chart', `${page}: ${title}`, () => s.hover(box.x, box.y),
      { done: `document.querySelector('.chart-tooltip:popover-open')`, timeout: 2000, quiet: 50, widget: title });
  }
  await s.hover(5, 5);
}
