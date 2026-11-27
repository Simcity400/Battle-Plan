import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import * as controls from './controls.mjs';

const source = readFileSync(new URL('./controls.mjs', import.meta.url), 'utf8');

function el(tag, { attrs = {}, className = '', text = '', children = [], rect = { left: 0, top: 0, width: 40, height: 20 }, disabled = false, checked = false, value = '', max = '' } = {}) {
  const classes = new Set(className.split(/\s+/).filter(Boolean));
  const attrMap = new Map(Object.entries(attrs).map(([k, v]) => [k.toLowerCase(), String(v)]));
  const node = {
    tagName: tag.toUpperCase(),
    className,
    disabled,
    checked,
    value,
    max,
    parentElement: null,
    children: [],
    childNodes: [],
    classList: { contains: name => classes.has(name), add: name => classes.add(name), remove: name => classes.delete(name) },
    getAttribute: name => (attrMap.has(name.toLowerCase()) ? attrMap.get(name.toLowerCase()) : null),
    setAttribute: (name, val) => { attrMap.set(name.toLowerCase(), String(val)); },
    hasAttribute: name => attrMap.has(name.toLowerCase()),
    getClientRects: () => (rect ? [rect] : []),
    getBoundingClientRect: () => rect ?? { left: 0, top: 0, width: 0, height: 0 },
    get textContent() {
      return text + this.children.map(c => c.textContent).join('');
    },
    matches(selector) {
      return splitTop(selector, ',').some(part => matchCompound(this, part.trim()));
    },
    closest(selector) {
      for (let cur = this; cur; cur = cur.parentElement) if (cur.matches(selector)) return cur;
      return null;
    },
    querySelector(selector) {
      return this.querySelectorAll(selector)[0] ?? null;
    },
    querySelectorAll(selector) {
      const out = [];
      const walk = root => {
        for (const child of root.children) {
          if (child.matches(selector)) out.push(child);
          walk(child);
        }
      };
      walk(this);
      return out;
    },
  };
  if (text) node.childNodes.push({ textContent: text });
  for (const child of children) {
    child.parentElement = node;
    node.children.push(child);
    node.childNodes.push(child);
  }
  return node;
}

function splitTop(selector, sep) {
  const out = [];
  let depth = 0, cur = '';
  for (const ch of selector) {
    if (ch === '(' || ch === '[') depth++;
    else if (ch === ')' || ch === ']') depth--;
    if (depth === 0 && ch === sep) { out.push(cur); cur = ''; }
    else cur += ch;
  }
  out.push(cur);
  return out;
}

function tokenizeCompound(selector) {
  const tokens = [];
  let depth = 0, cur = '';
  for (let i = 0; i < selector.length; i++) {
    const ch = selector[i];
    if (ch === '(' || ch === '[') depth++;
    else if (ch === ')' || ch === ']') depth--;
    if (depth === 0 && (ch === '>' || /\s/.test(ch))) {
      if (cur) { tokens.push(cur); cur = ''; }
      if (ch === '>') {
        if (tokens.at(-1) === ' ') tokens.pop();
        tokens.push('>');
      } else if (tokens.length && tokens.at(-1) !== '>' && tokens.at(-1) !== ' ') tokens.push(' ');
    } else cur += ch;
  }
  if (cur) tokens.push(cur);
  if (tokens.at(-1) === ' ') tokens.pop();
  return tokens;
}

function matchSimple(node, token) {
  let rest = token;
  if (rest === '*') return true;
  if (rest.includes(':popover-open')) {
    if (!node.hasAttribute('data-popover-open')) return false;
    rest = rest.replaceAll(':popover-open', '');
  }
  const notMatches = [...rest.matchAll(/:not\(([^)]+)\)/g)];
  for (const m of notMatches) {
    if (matchCompound(node, m[1].trim())) return false;
    rest = rest.replace(m[0], '');
  }
  const tagMatch = rest.match(/^[a-z][a-z0-9-]*/i);
  if (tagMatch) {
    if (node.tagName.toLowerCase() !== tagMatch[0].toLowerCase()) return false;
    rest = rest.slice(tagMatch[0].length);
  }
  for (const attr of rest.matchAll(/\[([^\]=^]+)(\^)?=(?:"([^"]*)"|([^\]]*))\]|\[([^\]=]+)\]/g)) {
    const [, nameEq, startsWith, qv, uv, nameOnly] = attr;
    const name = nameEq ?? nameOnly;
    if (!node.hasAttribute(name)) return false;
    if (nameEq !== undefined) {
      const expected = qv ?? uv;
      const actual = node.getAttribute(name);
      if (startsWith ? !actual.startsWith(expected) : actual !== expected) return false;
    }
    rest = rest.replace(attr[0], '');
  }
  for (const cls of rest.matchAll(/\.([a-z0-9_-]+)/gi)) {
    if (!node.classList.contains(cls[1])) return false;
  }
  return true;
}

function matchCompound(node, selector) {
  const parts = tokenizeCompound(selector);
  let cur = node;
  let i = parts.length - 1;
  if (!matchSimple(cur, parts[i])) return false;
  i--;
  while (i >= 0) {
    const comb = parts[i];
    const prevSel = parts[i - 1];
    if (comb === '>') {
      cur = cur.parentElement;
      if (!cur || !matchSimple(cur, prevSel)) return false;
    } else {
      let found = null;
      for (let p = cur.parentElement; p; p = p.parentElement) {
        if (matchSimple(p, prevSel)) { found = p; break; }
      }
      if (!found) return false;
      cur = found;
    }
    i -= 2;
  }
  return true;
}

test('ownedMenuExpr enforces popovertarget ownership and waits for React mount and lazy settings', () => {
  assert.equal(typeof controls.ownedMenuExpr, 'function', 'ownedMenuExpr must be exported');
  const menuA = el('div', { className: 'ui-popover widget-settings-menu', attrs: { id: 'menu-a', 'data-popover-open': '' } });
  const loadingP = el('p', { attrs: { role: 'status' }, text: 'Loading settings…' });
  const menuB = el('div', { className: 'ui-popover widget-settings-menu', attrs: { id: 'menu-b', 'data-popover-open': '' }, children: [loadingP] });
  const trigA = el('button', { attrs: { 'data-widget-settings': '', popovertarget: 'menu-a', 'aria-expanded': 'true' } });
  const trigB = el('button', { attrs: { 'data-widget-settings': '', popovertarget: 'menu-b', 'aria-expanded': 'false' } });
  const artA = el('article', { className: 'dash-widget', attrs: { 'data-widget': 'w-a' }, children: [trigA] });
  const artB = el('article', { className: 'dash-widget', attrs: { 'data-widget': 'w-b' }, children: [trigB] });
  const doc = {
    querySelector: sel => {
      if (sel.includes('w-a')) return trigA;
      if (sel.includes('w-b')) return trigB;
      return null;
    },
    getElementById: id => (id === 'menu-a' ? menuA : id === 'menu-b' ? menuB : null),
  };
  const evalOwned = widget => new Function('document', `return ${controls.ownedMenuExpr(widget)}`)(doc);

  assert.equal(evalOwned({ key: 'w-a' }), menuA, 'owned open menu resolves');
  assert.equal(evalOwned({ key: 'w-b' }), null, 'trigger with aria-expanded=false is not ready even if :popover-open');
  trigB.setAttribute('aria-expanded', 'true');
  assert.equal(evalOwned({ key: 'w-b' }), null, 'menu still showing Loading settings… is not ready');
  menuB.children.length = 0;
  menuB.children.push(el('div', { className: 'map-menu-section', children: [el('p', { className: 'map-menu-note', attrs: { role: 'status' }, text: 'Saved' })] }));
  assert.equal(evalOwned({ key: 'w-b' }), menuB, 'mounted settings menu with nested map-menu-note status resolves');
});

test('closeMenu clears an open tooltip before dismissing the settings menu', async () => {
  assert.equal(typeof controls.closeMenu, 'function', 'closeMenu must be exported');
  let tooltipOpen = true;
  let menuOpen = true;
  const actions = [];
  const fakeSession = {
    hover: async (x, y) => { actions.push(['hover', x, y]); },
    frames: async () => {},
    holds: async expr => {
      if (expr.includes('app-tooltip')) return tooltipOpen;
      if (expr.includes('widget-settings-menu')) return menuOpen;
      return false;
    },
    escape: async () => {
      actions.push(['escape']);
      if (tooltipOpen) { tooltipOpen = false; return; }
      menuOpen = false;
    },
  };
  await controls.closeMenu(fakeSession);
  assert.equal(tooltipOpen, false, 'tooltip dismissed');
  assert.equal(menuOpen, false, 'settings menu dismissed even after tooltip ate the first Escape');
  assert.equal(actions.filter(([k]) => k === 'escape').length, 2);
});

test('graphicsStateInMenu handles .map-appearance inside both .widget-settings-menu and .map-controls-menu', () => {
  assert.equal(typeof controls.graphicsStateInMenu, 'function', 'graphicsStateInMenu must be exported');
  const loadingAppearance = el('div', { className: 'map-appearance' });
  const dashMenuLoading = el('div', { className: 'widget-settings-menu', attrs: { 'data-popover-open': '' }, children: [loadingAppearance] });
  assert.equal(controls.graphicsStateInMenu(dashMenuLoading), 'loading');

  const readyAppearance = el('div', { className: 'map-appearance', children: [el('div', { className: 'graphics-inheritance' })] });
  const dashMenuReady = el('div', { className: 'widget-settings-menu', attrs: { 'data-popover-open': '' }, children: [readyAppearance] });
  assert.equal(controls.graphicsStateInMenu(dashMenuReady), 'ready');

  const failedAppearance = el('div', { className: 'map-appearance', children: [el('p', { attrs: { role: 'alert' }, text: 'boom' })] });
  const mapMenuFailed = el('div', { className: 'map-controls-menu', attrs: { 'data-popover-open': '' }, children: [failedAppearance] });
  assert.equal(controls.graphicsStateInMenu(mapMenuFailed), 'failed: boom');

  assert.equal(controls.graphicsStateInMenu(el('div', { className: 'widget-settings-menu' })), 'none');
});

test('CHART_SURFACE matches FamilyHost time-chart canvas and legacy .live-toggle branch is removed', () => {
  assert.equal(typeof controls.CHART_SURFACE, 'string', 'CHART_SURFACE must be exported');
  const canvas = el('canvas', { className: 'fh-canvas', attrs: { 'data-surface': 'time-chart' } });
  const extent = el('div', { className: 'fh-extent', children: [canvas] });
  const content = el('div', { className: 'dash-widget-content', children: [extent] });
  const article = el('article', { className: 'dash-widget', children: [content] });
  assert.equal(article.querySelector(controls.CHART_SURFACE), canvas);
  assert.doesNotMatch(source, /\.live-toggle/, 'dead .live-toggle branch must be removed');
});

test('discoverInMenu classifies video export, TableSettingsField, and RegionsField with zero unsupported controls and stable identities', () => {
  assert.equal(typeof controls.discoverInMenu, 'function', 'discoverInMenu must be exported');
  const namesToggle = el('button', {
    className: 'table-settings-names-toggle is-on',
    attrs: { role: 'switch', 'aria-checked': 'true', 'aria-label': 'Show names for Country' },
    text: 'Names',
  });
  const eyeBtn = el('button', {
    className: 'table-settings-eye',
    attrs: { 'aria-pressed': 'true', 'aria-label': 'Hide Army' },
  });
  const colRow = el('div', {
    className: 'table-settings-column',
    children: [el('span', { className: 'table-settings-col-name', text: 'Army' }), namesToggle, eyeBtn],
  });
  const colsGroup = el('div', {
    className: 'setting-row setting-row-stacked',
    attrs: { role: 'group', 'aria-label': 'Columns' },
    children: [
      el('span', { className: 'setting-label', text: 'Columns' }),
      el('button', { className: 'table-settings-link', text: 'Reset to default order' }),
      colRow,
    ],
  });
  const tableField = el('div', {
    className: 'table-settings-field',
    attrs: { 'data-setting': 'tableSettings' },
    children: [
      el('div', { className: 'setting-row table-settings-actions-row', children: [el('button', { className: 'table-settings-link', text: 'Fit all columns' })] }),
      colsGroup,
    ],
  });
  const regionsField = el('div', {
    className: 'setting-row setting-row-stacked',
    attrs: { 'data-setting': 'regions' },
    children: [
      el('span', { className: 'setting-label', text: 'Strategic regions' }),
      el('div', {
        className: 'regions-field-actions',
        children: [
          el('button', { className: 'button small quiet', attrs: { 'aria-haspopup': 'dialog' }, text: 'Add region' }),
          el('button', { className: 'button small quiet', text: 'Reset to automatic' }),
        ],
      }),
    ],
  });
  const mapSection = el('div', {
    className: 'map-menu-section',
    children: [
      el('button', { className: 'map-menu-action', text: 'Export map image' }),
      el('button', { className: 'map-menu-action', text: 'Export map video…' }),
    ],
  });
  const footer = el('div', {
    className: 'widget-settings-footer',
    children: [el('button', { className: 'button small', text: 'Export widget video…' })],
  });
  const menu = el('div', {
    className: 'ui-popover widget-settings-menu',
    attrs: { 'data-popover-open': '' },
    children: [tableField, regionsField, mapSection, footer],
  });

  const found = controls.discoverInMenu(menu);
  assert.deepEqual(found.filter(c => c.kind === 'unsupported'), [], 'no controls should be unsupported');

  const sw = found.find(c => c.kind === 'switch');
  assert.equal(sw?.label, 'Show names for Country');
  assert.equal(sw?.state, true);

  const eyeBefore = found.find(c => c.kind === 'press');
  assert.equal(eyeBefore?.label, 'Army column');
  assert.equal(eyeBefore?.state, true);
  eyeBtn.setAttribute('aria-pressed', 'false');
  eyeBtn.setAttribute('aria-label', 'Show Army');
  const eyeAfter = controls.discoverInMenu(menu).find(c => c.kind === 'press');
  assert.equal(eyeAfter?.label, eyeBefore.label, 'table-settings-eye label must stay stable when toggled');
  assert.equal(eyeAfter?.state, false);

  assert.deepEqual(found.filter(c => c.kind === 'table-action').map(c => c.label), ['Fit all columns', 'Reset to default order']);
  assert.equal(found.filter(c => c.kind === 'regions').length, 1);
  assert.equal(found.filter(c => c.kind === 'regions-reset').length, 1);
  assert.equal(found.filter(c => c.kind === 'export').length, 1);
  assert.deepEqual(found.filter(c => c.kind === 'video-export').map(c => c.label), ['Export map video…', 'Export widget video…']);
});

test('pickOtherCountry selects from the recording without hardcoding Italy', () => {
  assert.equal(typeof controls.pickOtherCountry, 'function', 'pickOtherCountry must be exported');
  assert.doesNotMatch(source, /'Italy'|"Italy"/, 'country picker must not hardcode Italy');

  const moddedRows = [
    { name: 'United States', tag: 'USA', selected: true },
    { name: 'United Kingdom', tag: 'ENG', selected: false },
    { name: 'Republic of China', tag: 'CHI', selected: false },
  ];
  const pick1 = controls.pickOtherCountry(moddedRows, moddedRows[0]);
  assert.equal(pick1.row.tag, 'ENG');
  assert.equal(pick1.query, 'United Kingdom');

  const syntheticTagOnly = [
    { name: '', tag: 'AAA', selected: true },
    { name: '', tag: 'BBB', selected: false },
  ];
  const pick2 = controls.pickOtherCountry(syntheticTagOnly, syntheticTagOnly[0]);
  assert.equal(pick2.row.tag, 'BBB');
  assert.equal(pick2.query, 'BBB');

  const sharedPrefix = [
    { name: 'Alpha', tag: 'Z01', selected: true },
    { name: 'Alpha Republic', tag: 'Z02', selected: false },
  ];
  const back = controls.countryQuery(sharedPrefix, sharedPrefix[0]);
  assert.equal(back, 'Z01', 'falls back to tag when name matches every row and cannot narrow');

  const sharedName = [
    { name: 'Union', tag: 'UN1', selected: true },
    { name: 'Union', tag: 'UN2', selected: false },
  ];
  const pickShared = controls.pickOtherCountry(sharedName, sharedName[0]);
  assert.equal(pickShared.row.tag, 'UN2');
  assert.equal(pickShared.query, 'UN2');
});

test('discoverInMenu handles saved region picks without unsupported controls and orders Reset widths before Fit all columns', () => {
  const tableField = el('div', {
    className: 'table-settings-field',
    attrs: { 'data-setting': 'tableSettings' },
    children: [
      el('div', {
        className: 'setting-row table-settings-actions-row',
        children: [
          el('button', { className: 'table-settings-link', text: 'Fit all columns' }),
          el('button', { className: 'table-settings-link', text: 'Reset widths' }),
        ],
      }),
    ],
  });
  const regionsField = el('div', {
    className: 'setting-row setting-row-stacked',
    attrs: { 'data-setting': 'regions' },
    children: [
      el('span', { className: 'setting-label', text: 'Strategic regions' }),
      el('div', {
        className: 'regions-field-list',
        children: [
          el('div', {
            className: 'regions-field-item',
            children: [
              el('span', { className: 'regions-field-label', text: 'Western Europe' }),
              el('button', { attrs: { 'aria-label': 'Move Western Europe up' }, disabled: true, text: '↑' }),
              el('button', { attrs: { 'aria-label': 'Move Western Europe down' }, disabled: false, text: '↓' }),
              el('button', { attrs: { 'aria-label': 'Remove Western Europe' }, text: '×' }),
            ],
          }),
          el('div', {
            className: 'regions-field-item',
            children: [
              el('span', { className: 'regions-field-label', text: 'Baltic Sea' }),
              el('button', { attrs: { 'aria-label': 'Move Baltic Sea up' }, disabled: false, text: '↑' }),
              el('button', { attrs: { 'aria-label': 'Move Baltic Sea down' }, disabled: true, text: '↓' }),
              el('button', { attrs: { 'aria-label': 'Remove Baltic Sea' }, text: '×' }),
            ],
          }),
        ],
      }),
      el('div', {
        className: 'regions-field-actions',
        children: [
          el('button', { className: 'button small quiet', attrs: { 'aria-haspopup': 'dialog' }, text: 'Add region' }),
          el('button', { className: 'button small quiet', text: 'Reset to automatic' }),
        ],
      }),
    ],
  });
  const menu = el('div', { className: 'ui-popover widget-settings-menu', attrs: { 'data-popover-open': '' }, children: [tableField, regionsField] });
  const found = controls.discoverInMenu(menu);
  assert.deepEqual(found.filter(c => c.kind === 'unsupported'), []);
  assert.deepEqual(found.filter(c => c.kind === 'table-action').map(c => c.label), ['Reset widths', 'Fit all columns']);
  const moves = found.filter(c => c.kind === 'regions-move');
  assert.deepEqual(moves.map(m => [m.label, m.disabled]), [
    ['Move Western Europe up', true],
    ['Move Western Europe down', false],
    ['Move Baltic Sea up', false],
    ['Move Baltic Sea down', true],
  ]);
  assert.deepEqual(found.filter(c => c.kind === 'regions-remove').map(r => r.label), ['Remove Western Europe', 'Remove Baltic Sea']);
});

test('regionActionPlan and readRegionChooserState preserve saved selections and fail on chooser load alerts', () => {
  const autoPlan = controls.regionActionPlan({
    automatic: true,
    picks: [],
    rows: [{ index: 0, name: 'Northern France', inChart: false }],
  });
  assert.deepEqual(autoPlan, { row: { index: 0, name: 'Northern France', inChart: false }, restore: 'reset' });

  const savedPlan = controls.regionActionPlan({
    automatic: false,
    picks: ['Western Europe', 'Baltic Sea'],
    rows: [
      { index: 0, name: 'Western Europe', inChart: true },
      { index: 1, name: 'Baltic Sea', inChart: true },
      { index: 2, name: 'Eastern Med', inChart: false },
    ],
  });
  assert.deepEqual(savedPlan, { row: { index: 1, name: 'Baltic Sea', inChart: true }, restore: 'readd' });

  const hiddenOnlyPlan = controls.regionActionPlan({
    automatic: false,
    picks: [],
    rows: [{ index: 0, name: 'Northern France', inChart: false }],
  });
  assert.equal(hiddenOnlyPlan, null, 'hidden-only selections are not mutated when exclusions cannot be restored');

  const alertPopover = el('div', {
    className: 'region-chooser-popover',
    children: [
      el('p', {
        className: 'region-chooser-empty',
        attrs: { role: 'alert', 'data-detail': 'IPC timeout' },
        text: 'Couldn’t load the regions. ',
        children: [el('button', { text: 'Retry' })],
      }),
    ],
  });
  assert.deepEqual(controls.readRegionChooserState(alertPopover), { kind: 'error', message: 'Couldn’t load the regions. (IPC timeout)' });

  const emptyPopover = el('div', {
    className: 'region-chooser-popover',
    children: [el('p', { className: 'region-chooser-empty', text: 'No regions in this recording' })],
  });
  assert.deepEqual(controls.readRegionChooserState(emptyPopover), { kind: 'empty', rows: [] });
});

test('IN_MENU scopes every comma-separated selector alternative inside open settings popovers', () => {
  const outsideSwitch = el('button', { attrs: { role: 'switch' } });
  const outsideVideo = el('button', { className: 'map-menu-action', text: 'Export map video…' });
  const insideSwitch = el('button', { attrs: { role: 'switch' } });
  const insideVideo = el('button', { className: 'map-menu-action', text: 'Export map video…' });
  const openMenu = el('div', { className: 'widget-settings-menu', attrs: { 'data-popover-open': '' }, children: [insideSwitch, insideVideo] });
  const root = el('div', { children: [outsideSwitch, outsideVideo, openMenu] });
  assert.deepEqual(root.querySelectorAll(controls.CONTROL.switch), [insideSwitch]);
  assert.deepEqual(root.querySelectorAll(controls.CONTROL['video-export']), [insideVideo]);
});

test('chartTooltips waits for offscreen lazy widgets to mount before querying the canvas', async () => {
  let mounted = false;
  const hovered = [];
  const steps = [];
  const fakeSession = {
    app: {
      evaluate: async expr => {
        if (expr.includes('article.dash-widget') && expr.includes('.map(')) return [{ key: 'w-off', title: 'Offscreen chart' }];
        if (expr.includes('scrollIntoView')) return undefined;
        if (expr.includes('getBoundingClientRect')) return mounted ? { x: 120, y: 80 } : null;
        return null;
      },
    },
    frames: async () => {},
    until: async expr => {
      if (expr.includes('stateOf')) mounted = true;
      return true;
    },
    hover: async (x, y) => { hovered.push([x, y]); },
    step: async (area, item, label, action) => {
      steps.push([area, item, label]);
      await action();
      return { pass: true };
    },
  };
  await controls.chartTooltips(fakeSession, 'Overview');
  assert.deepEqual(steps, [['controls', 'tooltips/chart', 'Overview: Offscreen chart']]);
  assert.ok(hovered.some(([x, y]) => x === 120 && y === 80), 'hovered the lazy-mounted canvas');
});

test('video-export waits for preview settlement or error and fails on premature dismissal', async () => {
  assert.match(source, /img\.export-video-preview-img,\s*\$\{dialog\}\s*\.export-video-error/);
  const { Coverage } = await import('./coverage.mjs');
  const fails = [];
  const widget = { key: 'w-vid', title: 'Video test', view: null };
  const control = { kind: 'video-export', index: 0, label: 'Export widget video…', context: '', nth: 0, disabled: false };
  const fakeSession = {
    tier: 'full',
    everFailed: () => false,
    holds: async expr => expr.includes('aria-expanded'),
    locate: async () => null,
    until: async () => true,
    escape: async () => {},
    click: async () => {},
    step: async (_a, _i, _l, action) => { await action(); return { pass: true, reasons: [] }; },
    fail: async (_a, _i, label, reason) => { fails.push({ label, reason }); },
    app: {
      evaluate: async expr => {
        if (expr.includes('discoverInMenu')) return [control];
        if (expr.includes('graphicsStateInMenu')) return 'none';
        if (expr.includes('export-video-preview-img')) return null;
        return null;
      },
    },
  };
  await controls.exercise(fakeSession, 'Overview', widget, control, { known: new Set(), cover: new Coverage() }, null);
  assert.equal(fails.length, 1);
  assert.match(fails[0].reason, /closed before its preview rendered/);
});

test('regions-remove, regions-reset, and chooser-closed exercise paths restore underlying selection or fail honestly', async () => {
  const { Coverage } = await import('./coverage.mjs');
  const catalog = [
    { id: 1, name: 'Western Europe' },
    { id: 2, name: 'Baltic Sea' },
    { id: 3, name: 'Eastern Med' },
  ];
  const makeHarness = ({ picks, hidden = [], chooserCloses = false, customCatalog = catalog }) => {
    let state = { picks: [...picks], hidden: [...hidden] };
    const encode = () => [...state.picks.map(id => `r:${id}`), ...state.hidden.map(id => `h:${id}`)].join(' ');
    const fails = [];
    const records = [];
    let chooserOpen = false;
    const widget = { key: 'w-reg', title: 'Air superiority', view: null };
    const renderFieldDom = () => {
      const hint = state.hidden.length ? `${state.hidden.length} removed from the most active` : 'Choose regions';
      const labelEl = el('span', {
        className: 'setting-label',
        attrs: { 'data-tooltip': hint },
        text: 'Strategic regions',
        children: [el('span', { className: 'sr-only', text: hint })],
      });
      const actions = [el('button', { className: 'button small quiet', attrs: { 'aria-haspopup': 'dialog' }, text: 'Add region' })];
      if (state.picks.length || state.hidden.length) actions.push(el('button', { className: 'button small quiet', text: 'Reset to automatic' }));
      const items = state.picks.map(id => el('div', {
        className: 'regions-field-item',
        children: [el('span', { className: 'regions-field-label', text: customCatalog.find(r => r.id === id).name })],
      }));
      return el('div', {
        className: 'setting-row setting-row-stacked',
        attrs: { 'data-setting': 'regions' },
        children: [labelEl, ...items, el('div', { className: 'regions-field-actions', children: actions })],
      });
    };
    const currentControls = () => {
      const out = [
        { kind: 'regions', index: 0, label: 'Strategic regions', context: 'regions', nth: 0, disabled: false },
      ];
      if (state.picks.length || state.hidden.length) {
        out.push({ kind: 'regions-reset', index: 0, label: 'Reset to automatic', context: 'regions', nth: 0, disabled: false });
      }
      state.picks.forEach((id, i) => {
        const name = customCatalog.find(r => r.id === id).name;
        out.push({ kind: 'regions-move', index: i * 2, label: `Move ${name} up`, context: 'regions', nth: 0, disabled: i === 0 });
        out.push({ kind: 'regions-move', index: i * 2 + 1, label: `Move ${name} down`, context: 'regions', nth: 0, disabled: i === state.picks.length - 1 });
        out.push({ kind: 'regions-remove', index: i, label: `Remove ${name}`, context: 'regions', nth: 0, disabled: false });
      });
      return out;
    };
    const fakeSession = {
      tier: 'full',
      everFailed: () => false,
      holds: async expr => {
        if (expr.includes('aria-expanded')) return true;
        if (expr.includes('not([aria-haspopup=dialog])')) return state.picks.length > 0 || state.hidden.length > 0;
        return false;
      },
      locate: async () => null,
      until: async () => true,
      escape: async () => { chooserOpen = false; },
      click: async (sel, opts = {}) => {
        if (sel.includes('not([aria-haspopup=dialog])')) { state = { picks: [], hidden: [] }; return; }
        if (sel.includes('aria-haspopup=dialog')) { chooserOpen = !chooserCloses; return; }
        if (sel.includes('Remove ')) {
          const name = opts.text.replace(/^Remove /, '');
          const id = state.picks.find(x => customCatalog.find(r => r.id === x).name === name);
          state = { picks: state.picks.filter(x => x !== id), hidden: state.hidden.includes(id) ? state.hidden : [...state.hidden, id] };
          return;
        }
        if (sel.includes('Move ')) {
          const up = opts.text.endsWith(' up');
          const name = opts.text.replace(/^Move /, '').replace(/ (up|down)$/, '');
          const id = state.picks.find(x => customCatalog.find(r => r.id === x).name === name);
          const idx = state.picks.indexOf(id);
          const next = [...state.picks];
          next.splice(idx, 1);
          next.splice(up ? idx - 1 : idx + 1, 0, id);
          state = { ...state, picks: next };
          return;
        }
        if (sel.includes('region-chooser-row')) {
          const entry = customCatalog[opts.index];
          if (state.picks.includes(entry.id)) {
            state = { picks: state.picks.filter(x => x !== entry.id), hidden: [...state.hidden, entry.id] };
          } else {
            state = { picks: [...state.picks, entry.id], hidden: state.hidden.filter(x => x !== entry.id) };
          }
          chooserOpen = false;
        }
      },
      step: async (_a, _i, _l, action) => { await action(); return { pass: true, reasons: [] }; },
      record: async entry => { records.push(entry); return entry; },
      fail: async (_a, _i, label, reason) => { fails.push({ label, reason }); },
      app: {
        evaluate: async expr => {
          if (expr.includes('discoverInMenu')) return currentControls();
          if (expr.includes('graphicsStateInMenu')) return 'none';
          if (expr.includes('readRegionFieldInDom')) return controls.readRegionFieldInDom(renderFieldDom());
          if (expr.includes('regions-field-label')) return state.picks.map(id => customCatalog.find(r => r.id === id).name);
          if (expr.includes('readRegionChooserState')) {
            if (!chooserOpen) return { kind: 'closed' };
            return { kind: 'ready', rows: customCatalog.map((r, index) => ({ index, name: r.name, inChart: state.picks.includes(r.id) })) };
          }
          if (expr.includes('region-chooser-row') && expr.includes('findIndex')) {
            const m = expr.match(/=== "([^"]+)"/);
            const wanted = m?.[1];
            return customCatalog.findIndex(r => !state.picks.includes(r.id) && r.name === wanted);
          }
          return null;
        },
      },
    };
    return { fakeSession, widget, fails, records, getState: () => ({ ...state, encoded: encode() }), currentControls };
  };

  const rem = makeHarness({ picks: [1, 2] });
  const removeB = rem.currentControls().find(c => c.label === 'Remove Baltic Sea');
  await controls.exercise(rem.fakeSession, 'Air', rem.widget, removeB, { known: new Set(), cover: new Coverage() }, null);
  assert.deepEqual(rem.fails, [], 'Remove B must reopen menu and restore [1, 2] with zero failures');
  assert.equal(rem.getState().encoded, 'r:1 r:2');

  const rst = makeHarness({ picks: [1, 2] });
  const resetBtn = rst.currentControls().find(c => c.kind === 'regions-reset');
  await controls.exercise(rst.fakeSession, 'Air', rst.widget, resetBtn, { known: new Set(), cover: new Coverage() }, null);
  assert.deepEqual(rst.fails, [], 'Reset must reopen menu and restore [1, 2] with zero failures');
  assert.equal(rst.getState().encoded, 'r:1 r:2');

  const hiddenOnly = makeHarness({ picks: [], hidden: [3] });
  const hiddenReset = hiddenOnly.currentControls().find(c => c.kind === 'regions-reset');
  await controls.exercise(hiddenOnly.fakeSession, 'Air', hiddenOnly.widget, hiddenReset, { known: new Set(), cover: new Coverage() }, null);
  assert.deepEqual(hiddenOnly.fails, []);
  assert.equal(hiddenOnly.getState().encoded, 'h:3', 'Reset reads SettingLabel tooltip/sr-only hint and leaves hidden exclusions intact');
  assert.ok(hiddenOnly.records.some(r => r.label.includes('hidden exclusions')));

  const dupCatalog = [
    { id: 1, name: 'Baltic Sea' },
    { id: 2, name: 'Baltic Sea' },
  ];
  const dupReset = makeHarness({ picks: [2], customCatalog: dupCatalog });
  const dupResetBtn = dupReset.currentControls().find(c => c.kind === 'regions-reset');
  await controls.exercise(dupReset.fakeSession, 'Air', dupReset.widget, dupResetBtn, { known: new Set(), cover: new Coverage() }, null);
  assert.deepEqual(dupReset.fails, []);
  assert.equal(dupReset.getState().encoded, 'r:2', 'Reset skips ambiguous duplicate labels before mutating r:2 into r:1');
  assert.ok(dupReset.records.some(r => r.label.includes('ambiguous')));

  const closedReset = makeHarness({ picks: [2], customCatalog: dupCatalog, chooserCloses: true });
  const closedResetBtn = closedReset.currentControls().find(c => c.kind === 'regions-reset');
  await controls.exercise(closedReset.fakeSession, 'Air', closedReset.widget, closedResetBtn, { known: new Set(), cover: new Coverage() }, null);
  assert.equal(closedReset.fails.length, 1, 'Reset fails closed when pre-mutation chooser closes');
  assert.equal(closedReset.getState().encoded, 'r:2', 'Reset must not mutate when chooser is unreadable');

  const closedRemove = makeHarness({ picks: [2], customCatalog: dupCatalog, chooserCloses: true });
  const closedRemoveBtn = closedRemove.currentControls().find(c => c.kind === 'regions-remove');
  await controls.exercise(closedRemove.fakeSession, 'Air', closedRemove.widget, closedRemoveBtn, { known: new Set(), cover: new Coverage() }, null);
  assert.equal(closedRemove.fails.length, 1, 'Remove fails closed when pre-mutation chooser closes');
  assert.equal(closedRemove.getState().encoded, 'r:2', 'Remove must not mutate when chooser is unreadable');

  const closedChooser = makeHarness({ picks: [], chooserCloses: true });
  const addBtn = closedChooser.currentControls().find(c => c.kind === 'regions');
  await controls.exercise(closedChooser.fakeSession, 'Air', closedChooser.widget, addBtn, { known: new Set(), cover: new Coverage() }, null);
  assert.equal(closedChooser.fails.length, 1);
  assert.match(closedChooser.fails[0].reason, /closed before its list could be read/);
});

test('chartTooltips fails when an offscreen widget stays loading past timeout and countryDoneExpr compiles and checks header text', async () => {
  const fails = [];
  const steps = [];
  const fakeSession = {
    app: {
      evaluate: async expr => {
        if (expr.includes('article.dash-widget') && expr.includes('.map(')) return [{ key: 'w-stuck', title: 'Stuck chart' }];
        return null;
      },
    },
    frames: async () => {},
    until: async () => false,
    hover: async () => {},
    step: async (...args) => { steps.push(args); return { pass: true }; },
    fail: async (_a, _i, label, reason) => { fails.push({ label, reason }); },
  };
  await controls.chartTooltips(fakeSession, 'Overview');
  assert.equal(steps.length, 0);
  assert.equal(fails.length, 1);
  assert.match(fails[0].reason, /stayed loading/);

  const trigger = '.header-workspace-bar button[aria-label^="Choose country"]';
  const expr = controls.countryDoneExpr(trigger, 'United Kingdom');
  const fn = new Function('document', `return (${expr});`);
  let pickerOpen = true;
  let strongText = 'Germany';
  const doc = {
    querySelector: sel => {
      if (sel.includes('.country-picker:popover-open')) return pickerOpen ? {} : null;
      if (sel.endsWith(' strong')) return { textContent: strongText };
      return { textContent: strongText };
    },
  };
  assert.equal(fn(doc), false, 'false while picker is still open');
  pickerOpen = false;
  assert.equal(fn(doc), false, 'false when header trigger stays stale');
  strongText = 'United Kingdom';
  assert.equal(fn(doc), true, 'true once picker closes and visible header matches');
});
