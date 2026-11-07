import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import type { CountryFlagLook } from '../../components/countryFlagPresentation';
import type { DiplomacyKind, DiplomacyPact } from '../../generated/dashboard';

type Pact = DiplomacyPact;

export type PactState = { state: 'present' } | { state: 'gone'; by: number } | { state: 'unknown'; since: number }
  /** Listed at the last prepared read (`since`); later hours are still being prepared. */
  | { state: 'preparing'; since: number };

/** Whether the recording lists a pact at `at`: listed through its last read (or still at the
 * last read); gone once a later read of its owner no longer lists it, dated by that read; and
 * unknown in between or where its owner was not read again, never assumed gone. Nothing is
 * said about the game's own state words. */
export function pactState(pact: Pact, at: number, through = Infinity): PactState {
  if (at <= pact.lastSeen) return { state: 'present' };
  if (pact.presentAtEnd) return at <= through ? { state: 'present' } : { state: 'preparing', since: through };
  // An episode a recording gap ended proves no absence: the next read may list it again.
  if (pact.endedBy) return { state: 'unknown', since: pact.lastSeen };
  return pact.goneBy !== null && pact.goneBy <= at ? { state: 'gone', by: pact.goneBy } : { state: 'unknown', since: pact.lastSeen };
}

/** Where a pact's presence starts: its stored start, else the first read that listed it (a bound). */
export const pactSince = (pact: Pact) => pact.startHour ?? pact.firstSeen;

/** Relation kinds other widgets show (lend-lease, market access and purchase contracts) and
 * the improve-relations action, which is not an agreement. */
const ELSEWHERE = new Set(['lend_lease', 'market_access_rights', 'equipment_purchase_contract_relation', 'improve_relation']);
/** An offer relation is its base relation read the other way round (the game links an
 * offered access from the second country to the first). */
const OFFERS: Readonly<Record<string, string>> = { offer_military_access: 'military_access', offer_docking_rights: 'docking_rights', offer_air_base_access: 'air_base_access' };

/** How each engine relation reads from the chosen country's side; any other kind (such as a
 * mod's) is listed after these under its game name, by direction. */
const KNOWN: readonly { kind: string; title: string; one: string; out?: (name: string) => string; in?: (name: string) => string; incomingFirst?: boolean }[] = [
  { kind: 'non_aggression_pact', title: 'Non-aggression pacts', one: 'Non-aggression pact' },
  { kind: 'guarantee', title: 'Guarantees', one: 'Guarantee', out: name => `${name} guarantees`, in: () => 'Guaranteed by' },
  { kind: 'military_access', title: 'Military access', one: 'Military access', out: name => `${name} has access in`, in: name => `Has access in ${name}` },
  { kind: 'docking_rights', title: 'Docking rights', one: 'Docking rights', out: name => `${name} docks in`, in: name => `Docks in ${name}` },
  { kind: 'air_base_access', title: 'Air base access', one: 'Air base access', out: name => `${name} flies from`, in: name => `Flies from ${name}` },
  { kind: 'embargo', title: 'Trade embargoes', one: 'Trade embargo', out: name => `${name} embargoes`, in: () => 'Embargoed by', incomingFirst: true },
  { kind: 'naval_blockade', title: 'Naval blockades', one: 'Naval blockade', out: name => `${name} blockades`, in: () => 'Blockaded by', incomingFirst: true },
];

/** One agreement between the chosen country and a partner: the listed episodes of one
 * relation (one kind, direction and stored start) joined. A relation ended and made again
 * has a new start, so one start held across a recording gap is the same relation throughout. */
export interface Agreement {
  key: string;
  kind: string;
  outgoing: boolean;
  partner: string | null;
  since: number;
  /** Seen without a stored start, so `since` is only when it was first listed. */
  sinceBound: boolean;
  /** A stored expiry after its start (a non-aggression pact's term). */
  expires: number | null;
  lastSeen: number;
  state: PactState;
  pact: Pact;
}

/** The chosen country's agreements at `at`: those listed by then, offers turned round,
 * kinds shown elsewhere left out, and one per relation. `through` is the last prepared hour. */
export function agreementsOf(pacts: readonly Pact[], tag: string, at: number, kinds: ReadonlySet<string> | null, through = Infinity): Agreement[] {
  const joined = new Map<string, { pacts: Pact[]; kind: string; outgoing: boolean; partner: string | null }>();
  for (const pact of pacts) {
    if (pact.firstSeen > at || ELSEWHERE.has(pact.kind)) continue;
    const kind = OFFERS[pact.kind] ?? pact.kind;
    if (kinds && !kinds.has(kind)) continue;
    const [first, second] = OFFERS[pact.kind] ? [pact.second, pact.first] : [pact.first, pact.second];
    if (first !== tag && second !== tag) continue;
    const outgoing = first === tag;
    const partner = outgoing ? second : first;
    const key = JSON.stringify([kind, outgoing, partner, pact.startHour ?? `seen:${pact.firstSeen}`]);
    const entry = joined.get(key) ?? joined.set(key, { pacts: [], kind, outgoing, partner }).get(key)!;
    entry.pacts.push(pact);
  }
  return [...joined].map(([key, entry]) => {
    const latest = entry.pacts.reduce((a, b) => b.lastSeen > a.lastSeen ? b : a);
    const since = Math.min(...entry.pacts.map(pactSince));
    const expiry = latest.endHour != null && latest.endHour > since ? latest.endHour : null;
    return { key, kind: entry.kind, outgoing: entry.outgoing, partner: entry.partner, since, sinceBound: latest.startHour == null,
      expires: expiry, lastSeen: latest.lastSeen, state: pactState(latest, at, through), pact: latest };
  });
}

/** `title` names one agreement; `mutual` has no direction (a pact between equals). */
export interface AgreementLaneInfo { agreement: Agreement; title: string; mutual: boolean }
export interface AgreementText {
  /** The chosen country's name, for side headings. */
  name: string;
  country: (tag: string | null, hour: number) => CountryFlagLook & { name: string };
  kindName: (kind: string) => string;
  colors: readonly string[];
}

const faded = (color: string) => /^#[0-9a-f]{6}$/i.test(color) ? `${color}66` : color;
/** Listed at the hour shown, or at the last prepared read with nothing known since. */
const live = (item: Agreement) => item.state.state === 'present' || item.state.state === 'preparing';
const order = (a: Agreement, b: Agreement) => Number(!live(a)) - Number(!live(b)) || a.since - b.since;

/** A section per kind (headed by how many are in force), split by direction where the
 * kind has one, and a lane per partner with its bar from start to end. A pact still listed
 * at the last known read runs open to `knownLastHour`: the branch end once prepared. */
export function agreementLanes(agreements: readonly Agreement[], ended: boolean, knownLastHour: number, text: AgreementText) {
  const lanes: TimelineLane[] = [];
  const info = new Map<string, AgreementLaneInfo>();
  const present = agreements.filter(item => ended || live(item));
  const known = new Map(KNOWN.map((entry, index) => [entry.kind, index]));
  const kinds = [...new Set(present.map(item => item.kind))].sort((a, b) => (known.get(a) ?? KNOWN.length) - (known.get(b) ?? KNOWN.length)
    || text.kindName(a).localeCompare(text.kindName(b)));
  kinds.forEach((kind, index) => {
    const spec = KNOWN.find(entry => entry.kind === kind);
    const title = spec?.title ?? text.kindName(kind);
    const items = present.filter(item => item.kind === kind);
    const color = text.colors[index % text.colors.length];
    const active = items.filter(item => item.state.state === 'present').length;
    lanes.push({ id: `kind:${kind}`, heading: true, rowHeight: 30, singleLine: true, separator: false, label: '',
      identity: { label: `${title} · ${active} in force` }, icons: false, names: false, spans: [] });
    const sides: { label: string | null; items: Agreement[] }[] = spec && !spec.out ? [{ label: null, items }]
      : [{ label: spec?.out?.(text.name) ?? `${text.name} → partner`, items: items.filter(item => item.outgoing) },
        { label: spec?.in?.(text.name) ?? `Partner → ${text.name}`, items: items.filter(item => !item.outgoing) }];
    if (spec?.incomingFirst) sides.reverse();
    for (const side of sides) {
      if (!side.items.length) continue;
      if (side.label) lanes.push({ id: `side:${kind}:${side.label}`, label: '', identity: { label: side.label }, heading: true, depth: 1, muted: true,
        labelStyle: 'side', separator: false, rowHeight: 18, singleLine: true, icons: false, names: false, spans: [] });
      for (const item of [...side.items].sort(order)) {
        const id = `agreement:${item.key}`;
        const look = text.country(item.partner, item.lastSeen);
        const listed = live(item);
        const span: TimelineSpan = { id: 'bar', firstHour: item.since, lastHour: item.pact.presentAtEnd ? knownLastHour : item.lastSeen,
          label: look.name, color: listed ? color : faded(color), barHeight: 8, openEnd: true };
        info.set(id, { agreement: item, title: spec?.one ?? title, mutual: !!spec && !spec.out });
        lanes.push({ id, label: '', identity: { label: look.name, countryFlag: look }, depth: 1, rowHeight: 22, singleLine: true,
          separator: false, icons: false, names: false, muted: !listed, spans: [span] });
      }
    }
  });
  return { lanes, info };
}

/** The kinds of relation the widget can show, named as the game names them. */
export function agreementKinds(kinds: readonly DiplomacyKind[]): { kind: string; name: string | null }[] {
  const named = new Map<string, string | null>();
  for (const kind of kinds) {
    if (ELSEWHERE.has(kind.kind)) continue;
    const base = OFFERS[kind.kind] ?? kind.kind;
    if (!named.get(base)) named.set(base, kind.name ?? named.get(base) ?? null);
  }
  return [...named].map(([kind, name]) => ({ kind, name: KNOWN.find(entry => entry.kind === kind)?.title ?? name }));
}

/** Whether a reply warning is about a kind the widget leaves out. */
export const aboutElsewhere = (warning: string) => [...ELSEWHERE].some(kind => warning.includes(`Relation ${kind}:`));
