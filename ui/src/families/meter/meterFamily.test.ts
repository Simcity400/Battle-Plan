import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { Canvas2D } from '../../charts/textWidth.ts';
import { createBoundedReadView, FutureDataError } from '../boundedView.ts';
import type { ArtProvider, FamilyTheme, IdentityProvider } from '../contract.ts';
import { missingArt, PENDING_ART, readyArt } from '../art/types.ts';
import { activeResearchProjection } from '../../widgets/active-research/activeResearchProjection.ts';
import { doctrineMasteryProjection } from '../../widgets/doctrine-mastery/doctrineMasteryProjection.ts';
import { decryptionProjection } from '../../widgets/decryption/decryptionProjection.ts';
import { deriveMeterAudit, deriveMeterMirror, deriveMeterNav, findMeterTooltip } from './model.ts';
import { hitMeterModel, layoutMeterModel } from './painter.ts';
import type { DecryptionReply, DoctrineMasteryReply, ResearchSlotsReply } from '../../generated/dashboard';
import type { ResearchData } from '../../widgets/research/types';

const THEME: FamilyTheme = {
  surface: '#14171d',
  text: '#eceff4',
  muted: '#94a0b2',
  accent: '#4da3ff',
  accentSoft: 'rgba(77, 163, 255, 0.16)',
  line: 'rgba(255, 255, 255, 0.1)',
  green: '#4cc38a',
  red: '#ff6b6f',
  amber: '#e5a93c',
  font: 'Inter, sans-serif',
  fontSize: { small: 11, body: 12 },
  dark: true,
};

function makeFakeCtx(): Canvas2D {
  return {
    font: '',
    measureText: (s: string) => ({ width: s.length * 7 }),
  } as unknown as Canvas2D;
}

function makeLive(values: Readonly<Record<string, number | bigint | undefined>>, missing: Readonly<Record<string, string>> = {}): LiveValues<number | bigint | string> {
  return {
    get: key => values[key],
    raw: key => {
      const v = values[key];
      return typeof v === 'bigint' ? v : typeof v === 'number' && Number.isSafeInteger(v) ? BigInt(v) : undefined;
    },
    missing: key => missing[key] ?? 'Not recorded',
  };
}

function makeIdentity(countries: Readonly<Record<string, { name: string; colour?: string; flag?: string; absent?: boolean }>>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const entry = countries[key];
      if (kind !== 'country' || !entry) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'ready', value: key },
          tag: { status: 'ready', value: key },
          name: { status: 'absent', reason: 'Name not recorded' },
          colour: { status: 'absent', reason: 'Colour not recorded' },
          flagChoice: { status: 'absent', reason: 'Flag not recorded' },
          validInterval: [atHour, atHour],
          nextChangeAt: Infinity,
        };
      }
      if (entry.absent) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'absent', reason: 'Country absent' },
          name: { status: 'absent', reason: 'Country absent' },
          colour: { status: 'absent', reason: 'Country absent' },
          flagChoice: { status: 'absent', reason: 'Country absent' },
          validInterval: [atHour, atHour],
          nextChangeAt: Infinity,
        };
      }
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: { status: 'ready', value: entry.name },
        colour: entry.colour ? { status: 'ready', value: entry.colour } : { status: 'absent', reason: 'No colour' },
        flagChoice: entry.flag ? { status: 'ready', value: entry.flag } : { status: 'absent', reason: 'No flag' },
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    },
  };
}

test('activeResearchProjection renders researching, idle, missing, stale caption, and daily nextChange', () => {
  const hour = 61320100;
  const history: ResearchData = {
    history: {
      coverage: [{ firstHour: 61320000, lastHour: 61320200 }],
      slots: [
        {
          index: 0,
          availability: [{ firstHour: 61320000, lastHour: 61320200 }],
          entries: [{ token: 'basic_machine_tools', databaseIndex: 1, firstObservedHour: 61320000, lastObservedHour: 61320100, endedByHour: null, outcomeObservedHour: null, outcome: 'active' }],
          states: [],
          progress: [],
        },
        {
          index: 1,
          availability: [{ firstHour: 61320000, lastHour: 61320200 }],
          entries: [],
          states: [],
          progress: [],
        },
        {
          index: 2,
          availability: [{ firstHour: 61320000, lastHour: 61320200 }],
          entries: [],
          states: [],
          progress: [],
        },
      ],
    },
    names: [{ token: 'basic_machine_tools', name: 'Basic Machine Tools' }],
    folders: [{ key: 'industry_folder', name: 'Industry' }],
    technologyFolders: [{ token: 'basic_machine_tools', databaseIndex: 1, folder: 'industry_folder' }],
    warnings: [],
    grants: { nextId: null, unavailable: null, grants: [] },
  };
  const reply: ResearchSlotsReply = {
    tag: 'GER',
    readHour: hour,
    missing: null,
    warnings: [],
    slots: [
      {
        index: 0,
        state: {
          firstHour: hour - 48,
          lastHour: hour,
          token: 'basic_machine_tools',
          databaseIndex: 1,
          entryFirstObservedHour: 61320000,
          bankedDaysRaw: '0',
          aheadReductionYearsRaw: null,
          details: { remainingDays: 92, modifiers: null },
        },
        progress: { firstHour: hour, lastHour: hour, entryFirstObservedHour: 61320000, progressRaw: '4500000', baseDaysRaw: '10000000' },
        definition: null,
      },
      {
        index: 1,
        state: {
          firstHour: hour,
          lastHour: hour,
          token: null,
          databaseIndex: null,
          entryFirstObservedHour: null,
          bankedDaysRaw: '1400000',
          aheadReductionYearsRaw: null,
          details: null,
        },
        progress: null,
        definition: null,
      },
    ],
    grants: { nextId: null, unavailable: null, grants: [] },
  };

  const art: ArtProvider = {
    resolve: handle => handle.choice === 'basic_machine_tools'
      ? readyArt('asset://tech/basic_machine_tools.png')
      : missingArt('Art missing from the current installation'),
  };

  const view = createBoundedReadView({
    hour,
    panels: new Map([['main', reply]]),
    live: new Map([['main', makeLive({ [liveKey('0', 'progress')]: 0.68 })]]),
    art,
  });

  const model = activeResearchProjection.project(view, { history, countryName: 'Germany' });
  assert.equal(model.layout, 'rows');
  assert.equal(model.summary, null);
  assert.equal(model.sections?.length, 1);
  const sec = model.sections![0]!;
  assert.equal(sec.title, 'Researching');
  assert.equal(sec.count, '1 of 3 slots');
  assert.ok(sec.rightCaption?.startsWith('As of '), 'shows single As of caption when reading >= 24h old');
  assert.equal(sec.rows?.length, 3);
  assert.equal(sec.rows![0]!.name, 'Basic Machine Tools');
  assert.equal(sec.rows![0]!.subtitle, 'Industry');
  assert.equal(sec.rows![0]!.figure, '68%');
  assert.equal(sec.rows![0]!.secondary, '90 days left');
  assert.equal(sec.rows![0]!.art?.result.status, 'ready');
  assert.equal(sec.rows![1]!.name, 'Idle');
  assert.equal(sec.rows![1]!.secondary, '14 days saved');
  assert.equal(sec.rows![2]!.name, 'Not recorded');
  assert.equal(sec.rows![2]!.figureMissing, true);
  assert.equal(activeResearchProjection.nextChange?.(view, { history, countryName: 'Germany' }), hour + 24);

  const unreadable = createBoundedReadView({
    hour,
    panels: new Map([['main', { ...reply, slots: [], missing: 'Research table block 100 unreadable: checksum' }]]),
  });
  const missingModel = activeResearchProjection.project(unreadable, { history, countryName: 'Germany' });
  assert.equal(missingModel.emptyKind, 'unavailable');
  assert.equal(missingModel.empty, 'Research table block 100 unreadable: checksum');
});

test('doctrineMasteryProjection builds threshold meters, missing mastery, empty track, and folder filter', () => {
  const hour = 61320100;
  const reply: DoctrineMasteryReply = {
    warnings: [],
    folders: [
      {
        ordinal: 0,
        id: 'land_doctrine',
        name: 'Land',
        icon: null,
        grand: { id: 'mobile_warfare', name: 'Mobile Warfare', icon: null },
        completed: [],
        tracks: [
          {
            ordinal: 0,
            name: 'Armour',
            sub: { id: 'armored_spearhead', name: 'Armored Spearhead', icon: null },
            unlocked: 2,
            rewards: [
              { id: 'r1', name: 'Breakthrough', threshold: '5000000' },
              { id: 'r2', name: 'Blitz', threshold: '5000000' },
              { id: 'r3', name: 'Modern Blitzkrieg', threshold: '10000000' },
            ],
          },
          {
            ordinal: 1,
            name: 'Infantry',
            sub: null,
            unlocked: 0,
            rewards: [],
          },
        ],
      },
      {
        ordinal: 1,
        id: 'mod_orbital_doctrine',
        name: 'Orbital Warfare',
        icon: null,
        grand: null,
        completed: [],
        tracks: [
          {
            ordinal: 0,
            name: 'Drop Pods',
            sub: { id: 'drop_assault', name: 'Drop Assault', icon: null },
            unlocked: 0,
            rewards: [{ id: 'o1', name: 'Shock Drop', threshold: '10000000' }],
          },
        ],
      },
    ],
  };

  const view = createBoundedReadView({
    hour,
    panels: new Map([['main', reply]]),
    live: new Map([['main', makeLive({
      [liveKey('0|0', 'mastery')]: 12400000n,
      [liveKey('0|0', 'effective')]: 150000n,
      [liveKey('0|0', 'daily')]: 100000n,
      [liveKey('0|0', 'bank')]: 0n,
      [liveKey('0|1', 'bank')]: 2000000n,
    }, {
      [liveKey('1|0', 'mastery')]: 'Doctrine mastery not recorded',
    })]]),
  });

  const model = doctrineMasteryProjection.project(view, { folders: [], countryName: 'Germany' });
  assert.equal(model.layout, 'columns');
  assert.equal(model.summary, '2 tracks active');
  assert.equal(model.sections?.length, 2);
  assert.equal(model.sections![0]!.title, 'Land \u00b7 Mobile Warfare');
  const armour = model.sections![0]!.rows![0]!;
  assert.equal(armour.name, 'Armored Spearhead');
  assert.equal(armour.subtitle, 'Step 3 of 3 \u00b7 Modern Blitzkrieg');
  assert.equal(armour.bar.kind, 'threshold');
  if (armour.bar.kind === 'threshold') {
    assert.equal(armour.bar.ticks.length, 3);
    assert.equal(armour.bar.ticks[0]!.reached, true);
    assert.equal(armour.bar.ticks[2]!.reached, false);
  }
  assert.equal(armour.figure, '124 / 200');
  assert.equal(armour.secondary, '+1.5 a day');
  assert.equal(armour.compactFigure, '2/3');

  const emptyTrack = model.sections![0]!.rows![1]!;
  assert.equal(emptyTrack.name, 'No subdoctrine chosen');
  assert.equal(emptyTrack.bar.kind, 'none');
  assert.equal(emptyTrack.secondary, '20 banked');

  const missingTrack = model.sections![1]!.rows![0]!;
  assert.equal(missingTrack.figureMissing, true);
  assert.equal(missingTrack.figureMissingLabel, 'Not recorded');

  const filtered = doctrineMasteryProjection.project(view, { folders: ['mod_orbital_doctrine'], countryName: 'Germany' });
  assert.equal(filtered.sections?.length, 1);
  assert.equal(filtered.sections![0]!.title, 'Orbital Warfare');
});

test('decryptionProjection renders Ciphers and Who reads whom views via readView.who and readView.art', () => {
  const hour = 61320100;
  const reply: DecryptionReply = {
    missing: null,
    attemptsUnknown: null,
    warnings: [],
    pairs: [
      {
        owner: { id: 1, tag: 'ENG' },
        target: { id: 2, tag: 'GER' },
        thresholdRaw: '10000000',
        thresholdUnknown: null,
        bonusActive: 0,
        decrypting: 1,
        broken: true,
        started: { hour: 61310000, upperBound: false },
        brokenAt: { hour: 61319000, upperBound: false },
      },
      {
        owner: { id: 2, tag: 'GER' },
        target: { id: 3, tag: 'SOV' },
        thresholdRaw: '10000000',
        thresholdUnknown: null,
        bonusActive: 0,
        decrypting: 1,
        broken: false,
        started: { hour: 61318000, upperBound: false },
        brokenAt: null,
      },
      {
        owner: { id: 1, tag: 'ENG' },
        target: { id: 3, tag: 'SOV' },
        thresholdRaw: null,
        thresholdUnknown: 'Target strength not recorded',
        bonusActive: 0,
        decrypting: 1,
        broken: null,
        started: null,
        brokenAt: null,
      },
    ],
  };

  const identity = makeIdentity({
    ENG: { name: 'United Kingdom', colour: '#d64545', flag: 'ENG:democratic' },
    GER: { name: 'German Reich', colour: '#666b73', flag: 'GER:fascism' },
    SOV: { name: 'Soviet Union', colour: '#b83232', flag: 'SOV:communism' },
  });
  const art: ArtProvider = {
    resolve: handle => handle.choice === 'ENG:democratic'
      ? readyArt('asset://flags/ENG.png')
      : handle.choice === 'GER:fascism'
        ? PENDING_ART
        : missingArt('Flag missing'),
  };

  const view = createBoundedReadView({
    hour,
    panels: new Map([['main', reply]]),
    live: new Map([['main', makeLive({
      [liveKey('1:2', 'amount')]: 10000000,
      [liveKey('2:3', 'amount')]: 6200000,
      [liveKey('1:3', 'amount')]: undefined,
    })]]),
    identity,
    art,
  });

  const allFilter = { passing: null, unrecorded: [], bounds: [] };
  const ciphers = decryptionProjection.project(view, { view: 'ciphers', filter: allFilter, filterAll: true, side: 'decryptor' });
  assert.equal(ciphers.layout, 'ciphers');
  assert.equal(ciphers.summary, '1 broken \u00b7 2 decrypting');
  assert.equal(ciphers.sections?.length, 2);
  assert.equal(ciphers.sections![0]!.title, 'Decrypting');
  assert.equal(ciphers.sections![0]!.rows?.length, 2);
  assert.equal(ciphers.sections![0]!.rows![0]!.figure, '62%');
  assert.equal(ciphers.sections![0]!.rows![1]!.figureMissing, true);
  assert.equal(ciphers.sections![1]!.title, 'Cipher broken');
  assert.equal(ciphers.sections![1]!.chipLines?.length, 1);
  assert.equal(ciphers.sections![1]!.chipLines![0]!.ownerName, 'United Kingdom');
  assert.equal(ciphers.sections![1]!.chipLines![0]!.chips[0]!.targetName, 'German Reich');
  assert.ok(!ciphers.sections![1]!.chipLines![0]!.chips[0]!.dateText?.includes(':'), 'broken date omits hour');

  const matrix = decryptionProjection.project(view, { view: 'matrix', filter: allFilter, filterAll: true, side: 'decryptor' });
  assert.equal(matrix.layout, 'matrix');
  assert.equal(matrix.matrix?.cornerLabel, 'Reads');
  assert.equal(matrix.matrix?.rows.length, 2);
  assert.equal(matrix.matrix?.columns.length, 2);
  const engRow = matrix.matrix!.rows[0]!;
  assert.equal(engRow.ownerName, 'United Kingdom');
  assert.equal(engRow.totalText, 'Broken 1 \u00b7 Decrypting 1');
  assert.ok(engRow.cells.some(c => c.kind === 'broken'));
  assert.ok(engRow.cells.some(c => c.kind === 'missing'));

  const ctx = makeFakeCtx();
  const layout = layoutMeterModel(ctx, ciphers, 832, THEME);
  assert.ok(layout.hits.length >= 3);
  const firstHit = layout.hits[0]!;
  assert.equal(hitMeterModel(layout, firstHit.x + 2, firstHit.y + 2, 0, 0), `row:${firstHit.key}`);
  assert.ok(findMeterTooltip(ciphers, firstHit.key) !== null);
  assert.equal(deriveMeterNav(ciphers).length, 3);
  assert.equal(deriveMeterMirror(ciphers, null).length, 3);
  assert.equal(deriveMeterAudit(ciphers).length, 3);
});

test('bounded readView throws FutureDataError on future who() lookups', () => {
  const view = createBoundedReadView({ hour: 100 });
  assert.throws(() => view.who('country', 'GER', 101), FutureDataError);
});
