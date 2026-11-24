import { useMemo } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import { useChoiceHour } from './atHead';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from './groups';
import type { CountryGroupReply } from '../generated/dashboard';
import { balanceRows } from '../widgets/balance-of-power/balanceOfPowerData';
import { occupiedRows } from '../widgets/occupied-states/occupiedStatesData';
import type { OccupiedStatesReply } from '../widgets/occupied-states/occupiedStatesData';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
const PUSHED = 100;

class Clock {
  position = PUSHED;
  private readonly frames = new Set<() => void>();
  getSnapshot = () => ({ position: this.position, hour: this.position, fraction: 0, playing: false, hoursPerSecond: 1, sequence: 0, seekSequence: 0,
    bounds: { firstHour: 0, lastHour: 200 } });
  getHour = () => this.position;
  subscribe = () => () => {};
  subscribeFrame = (listener: () => void) => { this.frames.add(listener); return () => { this.frames.delete(listener); }; };
  seek(hour: number) { this.position = hour; for (const listener of [...this.frames]) listener(); }
}

const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
const tags = ['POL', 'GER', 'MOD', 'ENG'];
const groups: CountryGroupReply = { branchId: '1', warnings: [], humans: ['POL', 'GER', 'MOD'].map(tag => ({ tag, firstHour: 125, lastHour: 200 })),
  factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
  countryIdentities: tags.map((tag, slot) => ({ tag, identities: [{ firstHour: 0, lastHour: 200, slot, countryId: slot + 1 }] })),
  unobserved: { humans: [{ firstHour: 110, lastHour: 120 }], factions: [], wars: [] } };
const balances = { balances: [{ ordinal: 0, token: 'guild', name: null, range: null, trending: null, members: [{ tag: 'MOD' }] }], hour: PUSHED } as never;
const occupied = { states: [{ state: 1, occupier: 'GER', occupied: 'ENG' }] } as unknown as OccupiedStatesReply;

let clock = new Clock();
const entry = { hour: PUSHED, through: PUSHED };
const live = new Set<() => void>();
const watchLive = (changed: () => void) => { live.add(changed); return () => { live.delete(changed); }; };
const tellLive = () => { for (const changed of [...live]) changed(); };
let renders = 0;

function Panels() {
  renders++;
  const match = useMemo(() => countryMatch(humans, [], groups, 200), []);
  const at = useChoiceHour(clock as never, true, () => entry.hour, () => entry.through, countryMatchChangeHours(match.matches), watchLive);
  const unrecorded = (hour: number) => membershipUnrecordedAt(humans, groups, hour);
  const shown = (view: { rows: readonly { key: string }[]; membership: string | null }) =>
    view.rows.length ? view.rows.map(row => row.key).join(',') : view.membership ?? 'none';
  const balance = shown(balanceRows({ ...(balances as { balances: [] }), hour: at } as never, match.matches, unrecorded));
  const state = shown(occupiedRows(occupied, tag => match.matches(tag, at), 'occupier', unrecorded(at)));
  return <output data-at={at}>{JSON.stringify({ balance, state })}</output>;
}

let root: Root | null = null;
let host: HTMLDivElement | null = null;

export const fixture = {
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); entry.hour = PUSHED; entry.through = PUSHED; renders = 0;
    host = document.body.appendChild(document.createElement('div'));
    root = createRoot(host);
    root.render(<Panels />);
    for (let at = 0; at < 4; at++) await frame();
  },
  async play(hour: number) {
    clock.seek(hour);
    entry.through = hour;
    tellLive();
    for (let at = 0; at < 4; at++) await frame();
  },
  async land(hour: number) {
    clock.seek(hour);
    entry.hour = entry.through = hour;
    tellLive();
    for (let at = 0; at < 4; at++) await frame();
  },
  async rerender() {
    root!.render(<Panels />);
    for (let at = 0; at < 4; at++) await frame();
  },
  shown: () => ({ ...JSON.parse(host!.querySelector('output')!.textContent!), at: Number(host!.querySelector('output')!.dataset.at) }),
  renders: () => renders,
};
(window as unknown as { choiceHour: typeof fixture }).choiceHour = fixture;
