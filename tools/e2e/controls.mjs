// Control cases. Each case is { id, page, run }: `page` is the sidebar label it runs on ('*' for every
// page, null for whichever is open) and `run(s, page)` drives the app through `s`, a Session, wrapping
// each user action in `s.step('controls', id, label, action, options)`. A case that cannot find its
// control throws, and the suite reports it as failed with that reason. To add a control, add a case.
import { existsSync } from 'node:fs';
import { mkdir, unlink } from 'node:fs/promises';
import path from 'node:path';
import { sleep } from './cdp.mjs';
import { Coverage, controlBase, placementKey } from './coverage.mjs';
import { Session, TOOLTIP_DELAY_MS } from './session.mjs';

const OPEN_POPOVER = sel => `document.querySelector(${JSON.stringify(`${sel}:popover-open`)})`;
const CLOSED_POPOVER = sel => `!document.querySelector(${JSON.stringify(`${sel}:popover-open`)})`;
const LABELLED = label => `button[aria-label=${JSON.stringify(label)}]`;
const widgetSel = title => `article.dash-widget[aria-label=${JSON.stringify(title)}]`;

/** A click that must leave `done` holding. */
const press = (s, id, label, selector, done, options = {}) =>
  s.step('controls', id, label, () => s.click(selector, options.locate ?? {}), { done, ...options });
/** The revisit that puts a control back: reported beside its first use, which the rule judges. */
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
    await press(s, this.id, 'play', LABELLED('Play recording'), `document.querySelector('button[aria-label="Pause playback"]')`);
    // Paused once it is playing: the clock has moved.
    await s.until(`Number(document.querySelector('input.header-timeline').value) > ${was}`, 4000);
    await press(s, this.id, 'pause', LABELLED('Pause playback'), `document.querySelector('button[aria-label="Play recording"]')`);
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
    // The calendar must move: from the recording's first month, forward first, then back.
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
    await s.escape();
    await press(s, this.id, 'go', '.header-date button.button.primary', `!document.querySelector('.header-date.has-date-draft') && ${shown} !== ${JSON.stringify(before)}`, { timeout: 10_000 });
  } },
  { id: 'header/country-picker', page: 'Overview', async run(s) {
    const trigger = '.header-workspace-bar button[aria-label="Choose country"]';
    const was = await s.app.evaluate(`document.querySelector(${JSON.stringify(trigger)})?.textContent.trim() ?? null`);
    if (was === null) throw new Error('no country picker');
    const listed = `Number(document.querySelector('.country-picker:popover-open [role=option]')?.getAttribute('aria-setsize') ?? 0)`;
    const names = `[...document.querySelectorAll('.country-picker:popover-open [role=option] small')].map(e => e.previousElementSibling?.textContent.trim() ?? '')`;
    const narrowed = (query, all) => `(${listed}) > 0 && (${listed}) < ${all} && ${names}.some(name => name.includes(${JSON.stringify(query)}))`;
    // Every row a search leaves must match it, by name or tag, as the search box says it searches.
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
    // The country it shows, as the picker names it: the row it marks chosen, anywhere in its list.
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
    // That exact country, by its tag: two countries can share a name.
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
    await press(s, this.id, 'open', '.header-layout .select-trigger', OPEN_POPOVER('.select-menu'));
    await s.step('controls', this.id, 'close', () => s.escape(), { done: CLOSED_POPOVER('.select-menu') });
  } },
  { id: 'header/edit-catalogue-cancel', page: 'Overview', async run(s) {
    await press(s, this.id, 'edit', LABELLED('Edit layout'), `document.querySelector('button[aria-label="Done arranging"]')`);
    await press(s, this.id, 'open catalogue', LABELLED('Add widget'), OPEN_POPOVER('.widget-catalogue'));
    const cards = `document.querySelectorAll('.widget-catalogue:popover-open .widget-card').length`;
    const every = await s.app.evaluate(cards);
    await s.click('.widget-catalogue:popover-open input[type=search]');
    await s.step('controls', this.id, 'search catalogue', () => s.type('map'),
      { done: `(${cards}) > 0 && (${cards}) < ${every} && [...document.querySelectorAll('.widget-catalogue:popover-open .widget-card')].some(c => /map/i.test(c.textContent))` });
    await press(s, this.id, 'close catalogue', LABELLED('Close widget catalogue'), CLOSED_POPOVER('.widget-catalogue'));
    await press(s, this.id, 'cancel', LABELLED('Cancel layout changes'), `document.querySelector('button[aria-label="Edit layout"]')`);
  } },
  { id: 'timeline/drag', page: 'Overview', async run(s) {
    const line = await s.timeline();
    if (!line) throw new Error('no timeline');
    const at = f => ({ x: Session.timelineX(line, f), y: line.y });
    // Released at a fraction of the track, the clock must stand within two pixels' worth of hours of it.
    const near = f => `Math.abs(Number(document.querySelector('input.header-timeline').value) - ${line.min + f * (line.max - line.min)}) <= ${2 * (line.max - line.min) / (line.width - 14) + 1}`;
    await s.step('controls', this.id, 'drag 30% to 70%', () => s.drag(at(0.3), at(0.7), 40), { timeout: 8000, from: 'last', done: near(0.7) });
    await s.step('controls', this.id, 'drag 70% to 20%', () => s.drag(at(0.7), at(0.2), 40), { timeout: 8000, from: 'last', done: near(0.2) });
    for (let i = 1; i <= 3; i++) {
      const was = (await s.timeline()).value;
      await s.step('controls', this.id, `arrow right ${i}`, () => s.key('ArrowRight'), { timeout: 4000, done: `Number(document.querySelector('input.header-timeline').value) > ${was}`, control: 'arrow right' });
    }
  } },
  { id: 'layout/drag-resize', page: 'Overview', async run(s) {
    await press(s, this.id, 'edit', LABELLED('Edit layout'), `document.querySelector('button[aria-label="Done arranging"]')`);
    await s.app.evaluate(`document.querySelector('.dash-grid-scroll').scrollTo({ top: 0 })`);
    await s.frames(2);
    const grip = await s.need('.dash-drag-grip', { index: 1 });
    // The widget dragged must move, and the one resized must change size.
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
    // The grid snaps a resize to its columns and rows: the handle is wherever the snapped size left it.
    const grown = await s.need(handleOf);
    await s.step('controls', this.id, 'resize smaller', () => s.drag({ x: grown.x - 2, y: grown.y - 2 }, { x: handle.x - 2, y: handle.y - 2 }), { timeout: 6000, from: 'last',
      done: sized(`Math.abs(r.width - ${resized.width}) <= 4 && Math.abs(r.height - ${resized.height}) <= 4`), ...back('resize bigger') });
    // Saved as a new layout, then the default chosen again and the new one: switching layouts.
    await press(s, this.id, 'open save', LABELLED('Save layout'), OPEN_POPOVER('.layout-save-menu'));
    const before = await s.app.evaluate(`document.querySelectorAll('.layout-save-menu button').length`);
    if (!before) throw new Error('no save options');
    // A new layout needs a name of its own: the field starts with the current layout's.
    const name = `Acceptance ${Date.now()}`;
    await s.click('.layout-save-menu:popover-open input[aria-label="Layout name"]');
    await s.app.evaluate(`document.querySelector('.layout-save-menu:popover-open input[aria-label="Layout name"]').select()`);
    await s.type(name);
    await press(s, this.id, 'save as new', '.layout-save-menu:popover-open .layout-save-actions button.primary',
      `document.querySelector('button[aria-label="Edit layout"]') && document.querySelector('.header-layout .select-trigger')?.textContent.trim() === ${JSON.stringify(name)}`);
    for (const [label, index] of [['switch to first layout', 0], ['switch to saved layout', -1]]) {
      await press(s, this.id, `${label}: open`, '.header-layout .select-trigger', OPEN_POPOVER('.select-menu'));
      const options = await s.app.evaluate(`Number(document.querySelector('.select-menu:popover-open [role=option]')?.getAttribute('aria-setsize') ?? 0)`);
      if (options < 2) throw new Error(`saving as new left ${options} layouts`);
      const at = index < 0 ? options - 1 : index;
      if (!await s.app.evaluate(`__e2e.revealOption(${at})`)) throw new Error(`layout ${at + 1} of ${options} never showed in the list`);
      await press(s, this.id, label, option(at), CLOSED_POPOVER('.select-menu'), { timeout: 8000 });
    }
  } },
  { id: 'widget/settings', page: '*', run: settingsSweep },
  { id: 'widget/live-toggle', page: '*', async run(s, page) {
    const widgets = await s.app.evaluate(`[...document.querySelectorAll('article.dash-widget')].filter(a => a.querySelector('.live-toggle'))
      .map(a => ({ key: a.dataset.widget, title: a.getAttribute('aria-label'), live: a.querySelector('.live-toggle').getAttribute('aria-checked') }))`);
    for (const { key, title, live } of widgets) {
      const selector = `article.dash-widget[data-widget=${JSON.stringify(key)}] .live-toggle`;
      const is = value => `document.querySelector(${JSON.stringify(selector)})?.getAttribute('aria-checked') === ${JSON.stringify(value)}`;
      const other = live === 'true' ? 'false' : 'true';
      await press(s, this.id, `${page}: ${title} ${live === 'true' ? 'off' : 'on'}`, selector, is(other), { widget: key, control: `${page}: ${title}` });
      await press(s, this.id, `${page}: ${title} ${live === 'true' ? 'on' : 'off'}`, selector, is(live), { widget: key, ...back(`${page}: ${title}`) });
    }
  } },
  { id: 'tooltips/chart', page: '*', run: chartTooltips },
  { id: 'tooltips/app', page: 'Overview', async run(s) {
    // Hover intent: the app waits TOOLTIP_DELAY_MS on purpose, so the budget is that plus the rule.
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
/** The map's graphics settings are a draft until Apply, which the host saves and the viewer then draws. */
const DRAFT = `document.querySelector('.map-appearance-actions')`;
const APPLY = IN_MENU('.map-appearance-actions button.button:not(.quiet)');
/** Every saved graphics, sound, counter and visual-mod setting the open menu shows, as [setting, JSON value]: what its map
 * must report drawing with (`DashboardMapState.applied`). A value not known yet (null for a parent still loading) is left out. */
const MAP_EXPECTED = `[...document.querySelectorAll(${JSON.stringify(IN_MENU('[data-map-setting]'))})]
  .map(e => [e.getAttribute('data-map-setting'), e.getAttribute('data-map-value')]).filter(([setting, value]) => value !== 'null' || setting.startsWith('graphics.'))`;
/** The `index`th shown element of the open menu matching `selector`, as `Session.locate` counts them. */
const shown = (selector, index) => `[...document.querySelectorAll(${JSON.stringify(selector)})].filter(e => e.getClientRects().length)[${index}]`;

/** The controls the open settings menu shows now, counted as `Session.locate` counts them (shown ones only), each with what
 * identifies it across a change elsewhere in the menu. */
const discover = s => s.app.evaluate(`(() => {
  const menu = document.querySelector(${JSON.stringify(SETTINGS_MENU)}); if (!menu) return [];
  const text = e => e && [...e.childNodes].filter(n => !n.classList?.contains('sr-only')).map(n => n.textContent).join('');
  const out = [], label = e => (text(e.closest('label, .setting-row')?.querySelector('.setting-label, span')) || e.getAttribute('aria-label') || e.textContent || '').trim().slice(0, 60);
  const all = selector => [...menu.querySelectorAll(selector)].filter(e => e.getClientRects().length);
  const draft = e => !!e.closest('.map-appearance');
  const context = e => e.closest('[data-setting]')?.getAttribute('data-setting') ?? '';
  // A graphics setting set here, not inherited; one that inherits goes back to inheriting.
  const overridden = e => !!e.closest('.map-option-row, .map-mod-list')?.hasAttribute('data-overridden');
  all('input[role=switch]').forEach((e, i) => out.push({ kind: 'switch', index: i, label: label(e), context: context(e), draft: draft(e), overridden: overridden(e), disabled: e.disabled, state: e.checked }));
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
  // Controls with a case of their own, each named by what stays the same while its value changes.
  const fieldLabel = e => (e.closest('.date-time-field')?.querySelector(':scope > span')?.textContent ?? '').trim();
  const special = [
    ['check', 'input[type=checkbox]:not([role=switch])', e => label(e)],
    ['reset', '.map-appearance button.map-menu-action', e => e.textContent.trim()],
    ['search', 'input.map-mod-search', e => e.getAttribute('aria-label')],
    ['refresh', '.map-mod-tools > button', e => e.textContent.trim()],
    ['export', 'button.map-menu-action:not(.map-appearance *)', e => e.textContent.trim()],
    ['country', 'button.widget-country-trigger', e => { const named = e.getAttribute('aria-label') ?? '';
      // A picker of several countries, the map's counter filter's too, is named apart from how many it holds.
      return named.startsWith('Widget campaign:') ? 'Widget campaign' : named.startsWith('Up to ') || / add or remove$/.test(named) ? 'Add or remove a country' : named; }],
    ['pin', '.setting-row button.button.small', e => e.textContent.trim()],
    ['date', 'button.date-time-trigger', fieldLabel],
    // A faction by its id: its name is what the next control renames.
    ['colour', 'input[type=color]', e => 'Colour of faction ' + (e.closest('li[data-faction]')?.getAttribute('data-faction') ?? '?')],
    ['faction-name', 'input.faction-name', e => 'Display name of faction ' + (e.closest('li[data-faction]')?.getAttribute('data-faction') ?? '?')],
    ['icon', '[aria-label="Header icon"] button', e => e.getAttribute('aria-label')],
  ];
  const claimed = new Set();
  for (const [kind, selector, name] of special) all(selector).filter(e => kind !== 'pin' || /^Pin to now$/.test(e.textContent.trim()))
    .filter(e => kind !== 'export' || /^Export map image$/.test(e.textContent.trim()))
    .forEach((e, i) => claimed.add(e) && out.push({ kind, index: i, label: (name(e) || kind).slice(0, 80), context: context(e), draft: draft(e), overridden: overridden(e), disabled: e.disabled,
      ...(kind === 'check' ? { state: e.checked } : {}), ...(kind === 'country' ? { trigger: e.getAttribute('aria-label') } : {}) }));
  // Anything else a user can operate here has no case: it is reported by name, never passed over.
  const handled = 'input[role=switch], input[type=range], .segmented button, .select-trigger, .option-number input[type=number], label.field input:not([type]), '
    + '.content-set-picker > button[aria-haspopup=dialog], .chip-set button.chip, [aria-label="Widget size"] button';
  // The menu's own frame, and what the sweep itself uses to reach and restore controls.
  const frame = '.ui-popover-heading, summary, .map-option-inherit, .map-appearance-actions, .select-menu, .content-picker, .country-picker, .campaign-picker, .date-time-popover';
  all('button, input, select, textarea, [role=switch], [tabindex="0"]').filter(e => !e.matches(handled) && !claimed.has(e) && !e.closest(frame) && !e.matches('.folder-info'))
    .forEach(e => out.push({ kind: 'unsupported', label: label(e), what: e.tagName.toLowerCase() + (typeof e.className === 'string' && e.className ? '.' + e.className.split(' ')[0] : '') }));
  const seen = new Map();
  for (const c of out) { const name = c.kind + '|' + c.label + '|' + (c.context ?? ''); c.nth = seen.get(name) ?? 0; seen.set(name, c.nth + 1); }
  return out;
})()`);
const identity = c => `${c.kind}|${c.label}|${c.context ?? ''}|${c.nth}|${(c.buttons ?? c.chips)?.map(b => b.label).join('/') ?? ''}`;
/** A page expression for a control's state, as discovery reads it, at its current place in the menu. */
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

/** The kinds the quick tier's one setting prefers: those a user changes in one click. */
const QUICK_KINDS = ['switch', 'segment', 'select', 'check', 'chips'];

/** Every control in every widget's settings menu on `page`: each changed, and each change must show on the widget (or be
 * drawn by its map), then changed back. Controls a change reveals are exercised while it holds, each of their values once
 * across the menu's states (`Coverage`). Widgets are followed by their layout id, so a setting that renames one keeps it in
 * view. A placement that is the same case as one already exercised (`placementKey`) is not exercised again, unless a step
 * of it ever failed. The quick tier exercises one setting of the first placement of each widget kind. */
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
    // The map's graphics settings load after the menu opens; their controls are disabled until then.
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
      // The quick tier changes one setting and leaves it: its session ends with its profile, and its back is the full tier's.
      if (!quick) await reconcile(s, page, widget, before, c);
    }
    if (ctx.cover.skipped && !quick) await s.record({ area: 'controls', item: 'widget/settings', label: `${placement}: ${ctx.cover.skipped} values already exercised under another setting of its menu, not repeated`,
      control: `${placement}: open`, ms: null, pass: true, reasons: [], screenshot: false });
    await s.escape();
  }
}

/** Puts the menu back as `before` held it, after `exercised` was changed and restored. A setting the app reset when its
 * parent changed, as a user would see it reset, is set again; `exercised` itself not coming back fails by name, as does
 * any setting the sweep cannot set again. */
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

/** Sets one control, as `current` shows it now, to the state `target` held: unmeasured, through the user's own input. */
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
  if (target.draft && await s.holds(DRAFT)) { await s.click(APPLY); await s.until(`!${DRAFT}`, 8000); }
}

const widgetAt = key => `article.dash-widget[data-widget=${JSON.stringify(key)}]`;
/** Scrolls the widget whose settings are exercised into the grid's view, its top first: a widget below the fold draws
 * nothing there, so no setting could show on it (run 14: every Overview widget under the map, "no visible change"). */
const inView = (s, widget) => s.app.evaluate(`(() => { const a = document.querySelector(${JSON.stringify(widgetAt(widget.key))}); const grid = document.querySelector('.dash-grid-scroll');
  if (!a || !grid) return; const r = a.getBoundingClientRect(), g = grid.getBoundingClientRect();
  if (r.top < g.top || r.bottom > g.bottom) grid.scrollBy({ top: r.top - g.top, behavior: 'instant' }); })()`).then(() => s.frames(2));
const triggerOf = widget => `${widgetAt(widget.key)} [data-widget-settings], ${widgetAt(widget.key)} button[aria-label="Map settings"]`;
/** Opens the menu's collapsed sections, so the controls in them can be reached as a user reaches them. */
async function openSections(s) {
  for (let open = 0; open < 50; open++) {
    const summary = await s.locate(IN_MENU('details:not([open]) > summary'));
    if (!summary) return;
    await s.app.click(summary.x, summary.y);
    await s.frames(2);
  }
}
/** The map's graphics draft: none in this menu, loading (its controls disabled), ready, or failed with the menu's words. */
const graphicsState = `(() => { const section = document.querySelector('.map-controls-menu:popover-open .map-appearance');
  if (!section) return 'none'; const alert = section.querySelector('[role=alert]');
  return alert ? 'failed: ' + alert.textContent.trim() : section.querySelector('.graphics-inheritance') ? 'ready' : 'loading'; })()`;
/** Opens the menu if it closed, and its sections; returns the graphics draft's state once it has loaded or after 8 s. */
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

/** One control: changed and back. `ctx.known` holds the controls already visited, so each is visited once however deep it
 * is revealed, and `ctx.cover` which of its values were exercised, so each value runs once across the menu's states (and
 * again where its case ever failed); `under` names the change a revealed control is exercised under. Every step of one
 * control shares its name. */
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
  /** Whether to exercise `value` of this control now, the case named `label`: see `Coverage`. */
  const take = (value, label = name) => cover.take(controlBase(c), value, s.everFailed('controls', 'widget/settings', label));
  const step = (label, action, options = {}) => s.step('controls', 'widget/settings', label, action, { widget: key, ...id, ...options });
  /** A change to the control: shown on its widget or drawn by its map; a graphics draft is applied, then drawn. */
  const set = async (label, action, options = {}) => {
    if (!c.draft && !widget.view) return step(label, action, { changes, timeout: 3000, ...options });
    // An open menu hides the native map, which draws nothing while hidden: the setting must change its control, and the
    // map is then timed from the menu's dismissal to its first frame of the new request.
    if (!c.draft) {
      const since = s.app.lastSent('set_dashboard_maps');
      const was = (await discover(s)).find(x => identity(x) === identity(c))?.state;
      // The control's own state, read in the page each frame, until it differs from what it was.
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
    const drafted = await step(`${label}: draft`, action, { ...options, done: options.done ? `(${options.done}) && ${DRAFT}` : DRAFT });
    if (!drafted.pass && drafted.reasons.some(reason => reason.startsWith('could not act') || reason.startsWith('never finished'))) return drafted;
    await reopen(s, widget);
    const saved = await step(`${label}: apply`, () => s.click(APPLY), { check: options.check, done: `!${DRAFT}`, timeout: 8000, control: options.control ?? name });
    if (!saved.pass && saved.reasons.some(reason => reason.startsWith('could not act') || reason.startsWith('never finished'))) return saved;
    if (!widget.view) { await s.fail('controls', 'widget/settings', `${label}: drawn`, 'its menu saves map graphics, but its widget shows no map to draw them'); return saved; }
    // The map draws nothing behind its open menu: every saved row must then be what its next frame reports drawing with.
    const expected = await s.app.evaluate(MAP_EXPECTED);
    // Texture quality and visual mods rebuild the map's artwork: long enough to see that finish, judged on its time.
    await step(`${label}: drawn`, () => s.key('Escape'), { drew: { view: widget.view, expected }, timeout: 20_000, check: options.check, control: `${options.control ?? name}: drawn` });
    return saved;
  };
  /** Whether the control already holds what it had (a reset it revealed put it back): then nothing is put back. */
  const alreadyBack = async label => {
    const now = (await discover(s)).find(x => identity(x) === identity(c));
    if (!now || now.state === undefined || JSON.stringify(now.state) !== JSON.stringify(c.state) || (c.draft && !!now.overridden !== !!c.overridden)) return false;
    await s.record({ area: 'controls', item: 'widget/settings', label: `${label}: already back`, ...id, ms: null, pass: true, reasons: [], screenshot: false });
    return true;
  };
  /** Puts a control back: `reverse` undoes the change, but a graphics setting that inherited goes back to inheriting,
   * through its row's reset; a select inherits through its own Inherit option, which it had. */
  const restore = async (label, reverse) => {
    if (s.tier === 'quick') return;
    await reopenHere();
    if (await alreadyBack(label)) return;
    if (!c.draft || c.overridden || c.kind === 'select') return reverse();
    return set(label, async () => {
      // Visual mods inherit as a whole, through the list's own way back.
      if (c.kind === 'check') { await s.click(IN_MENU('.map-mods-inherit')); return; }
      const at = await s.app.evaluate(`(() => { const e = ${shown(CONTROL[c.kind], c.index)}?.closest('.map-option-row')?.querySelector('.map-option-inherit');
        if (!e) return null; e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
      if (!at) throw new Error('its row offers no way back to inheriting');
      await s.app.click(at.x, at.y);
    }, { check: false });
  };
  /** The controls a change revealed, exercised while it holds, and those they reveal in turn: each control once. */
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
  /** Reopens the menu and finds the control again by its identity: a change elsewhere can move it. */
  const reopenHere = async () => {
    await reopen(s, widget);
    const now = (await discover(s)).find(x => identity(x) === identity(c));
    if (!now) throw Object.assign(new Error('the control left its menu'), { gone: true });
    Object.assign(c, { index: now.index, group: now.group });
  };
  // A control with values takes each value as it comes to it; any other is one case.
  if (!['segment', 'select', 'chips'].includes(c.kind) && !take('')) return;
  await reopenHere();
  if (c.kind === 'unsupported') { await s.fail('controls', 'widget/settings', name, `not exercised: the sweep has no case for this control (${c.what})`); return; }
  // A control the app disables is shown as such, not used, as a user cannot use it.
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
    // Each option it offers, then the one it had; a disabled option says why, and is never chosen.
    /** The option's place in the list as it stands now, found by its label: a change elsewhere can rebuild the list. */
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
    // Each chip pressed or released, then back; one the app disables says why and is not used.
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
    // A step along it by keyboard, then the step back, which returns it to exactly the value it had.
    const [forth, backward] = c.value >= c.max ? ['ArrowLeft', 'ArrowRight'] : ['ArrowRight', 'ArrowLeft'];
    const nudge = key => async () => { await s.app.evaluate(`${shown(CONTROL.range, c.index)}.focus()`); await s.key(key); };
    await set(name, nudge(forth));
    await reopenHere();
    await restore(`${name} back`, () => set(`${name} back`, nudge(backward), { check: false }));
  } else if (c.kind === 'number') {
    // Up a step, then back to exactly what it held, blank (automatic) included, typed as a user would.
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
    // Every setting of its scope (or its visual mods) back to the parent's: saved, then drawn by the map.
    await set(name, () => s.click(IN_MENU('.map-appearance button.map-menu-action'), { index: c.index }));
  } else if (c.kind === 'search') {
    // A word of the first mod's name narrows the list to the mods whose names hold it; cleared, the whole list is back.
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
    // The installed mods read again; the button is usable again once they are, and no error shows.
    const button = IN_MENU('.map-mod-tools > button');
    await step(name, () => s.click(button), { done: `document.querySelector(${JSON.stringify(button)})?.disabled === false && !document.querySelector(${JSON.stringify(IN_MENU('.map-menu-error'))})`,
      awaits: false, timeout: 8000 });
  } else if (c.kind === 'export') {
    // As the map pictures export: the host's save dialog filled by the suite's native input, then the file must exist.
    if (!s.native || !s.work) { await s.fail('controls', 'widget/settings', name, "not exercised: the controls phase was given no native input for the host's save dialog"); return; }
    const folder = path.join(s.work, 'control-exports');
    await mkdir(folder, { recursive: true });
    const file = path.join(folder, `${name.replace(/[^\w.-]+/g, '_').slice(0, 80)}.png`);
    if (existsSync(file)) await unlink(file);
    await step(name, async () => {
      await s.click(IN_MENU('button.map-menu-action'), { text: 'Export map image', partial: true });
      await s.native.ask({ op: 'save_dialog', path: file, seconds: 20 });
    }, { awaits: false, check: false, timeout: 3000 });
    for (const end = Date.now() + 20_000; !existsSync(file) && Date.now() < end;) await sleep(100);
    if (existsSync(file)) await s.record({ area: 'controls', item: 'widget/settings', label: `${name}: written`, ...id, ms: null, pass: true, reasons: [], screenshot: false });
    else await s.fail('controls', 'widget/settings', `${name}: written`, 'the export wrote no image within 20 s');
  } else if (c.kind === 'country') {
    await exerciseCountry(s, c, name, id, step, set, reopenHere);
  } else if (c.kind === 'pin') {
    // Pinned again a day later than it holds: it must hold the new moment; then a day back, pinned again.
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
    // Another day chosen in its calendar, then its end of the campaign again, as a new date range starts.
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
    // A faction's colour through its native colour dialog, filled by the suite's native input; then its reset, or the colour it had.
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
    // Typed and left (a name saves on blur), timed from the Tab that leaves it; then the name it had, blank for the game's.
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
    // Typed, then Enter: timed from the Enter to the widget showing its new title.
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
    // A choice made in it must show on the widget; then what it had comes back: automatic, or the choice it held.
    const picker = '.content-picker:popover-open';
    const isOpen = `document.querySelector('${picker}')`;
    const openPicker = (label, options = {}) => step(`${name}: ${label}`, () => s.click(CONTROL['content-set'], { index: c.index }),
      { done: `document.querySelector('${picker} [role=option], ${picker} p.content-picker-empty:not([role])')`, control: `${name}: open`, ...options });
    const close = () => step(`${name}: close`, () => s.key('Escape'), { done: `!${isOpen}`, control: `${name}: close` });
    const opened = await openPicker('open');
    if (!opened.pass && opened.reasons.some(reason => reason.startsWith('could not act') || reason.startsWith('never finished'))) { await s.escape(); return; }
    // The whole list, by each row's value: labels can repeat, and the list shows only a window of its rows.
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
    // What it held must be what it holds again.
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

/** Moves the paused clock `hours` hours through the header's own buttons, the settings menu closed; why it could not. */
async function moveClock(s, hours) {
  await s.escape();
  const was = (await s.timeline())?.value;
  if (was === undefined) return 'there is no timeline';
  const at = `Number(document.querySelector('input.header-timeline')?.value)`;
  // Each click once the last has moved the clock, so none is lost.
  for (let step = 1; step <= Math.abs(hours); step++) {
    await s.click(LABELLED(hours > 0 ? 'Next hour' : 'Previous hour'));
    if (!await s.until(`${at} === ${was + Math.sign(hours) * step}`, 2000)) break;
  }
  const now = (await s.timeline()).value;
  return now === was + hours ? null : `the clock stood at ${now}, not ${was + hours}`;
}

/** A widget's country or campaign picker: another choice made, shown on the widget, then the one it had again. For several
 * countries, one added, then removed (or, at its limit, one removed, then added back). A campaign chosen by name may be the
 * recording the default shows: that choice shows in its picker's label. */
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

/** The header icon every widget of its kind shares: another icon, or colour, chosen in its picker must show in the widget's
 * header; then its default again through the picker's reset, or what it had. */
async function exerciseIcon(s, c, name, id, step, reopenHere, key) {
  const header = `document.querySelector(${JSON.stringify(`[data-widget-icon="${key}"]`)})`;
  const look = `(e => e ? e.innerHTML + '|' + (e.getAttribute('style') ?? '') : null)(${header})`;
  const before = await s.app.evaluate(look);
  const button = IN_MENU('[aria-label="Header icon"] button');
  if (c.label === 'Reset icon') {
    // Shown only while the kind has an icon of its own, which the sweep cannot know to give back.
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

/** The charts that show a tooltip for the point nearest the pointer, so their centre always shows one; decorative graphics
 * and charts whose tooltips belong to single marks are not this case's. */
const CHART_SURFACE = '.dash-widget-content .time-chart canvas';

/** Hovering each widget's time chart: how soon its tooltip shows. */
async function chartTooltips(s, page) {
  const titles = await s.app.evaluate(`[...document.querySelectorAll('article.dash-widget')].map(a => a.getAttribute('aria-label'))`);
  for (const title of titles) {
    const box = await s.app.evaluate(`(() => { const a = document.querySelector(${JSON.stringify(widgetSel(title))}); a?.scrollIntoView({ block: 'center' });
      const c = a?.querySelector(${JSON.stringify(CHART_SURFACE)}); if (!c) return null; const r = c.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
    if (!box) continue;
    await s.hover(5, 5);
    await s.until(`!document.querySelector('.chart-tooltip:popover-open')`, 1000);
    await s.frames(2);
    // The first hover is timed: a time chart shows the nearest point's tooltip at once.
    await s.step('controls', 'tooltips/chart', `${page}: ${title}`, () => s.hover(box.x, box.y),
      { done: `document.querySelector('.chart-tooltip:popover-open')`, timeout: 2000, quiet: 50, widget: title });
  }
  await s.hover(5, 5);
}
