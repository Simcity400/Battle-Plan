// Synthetic peace-deals replies shaped as the product writes them, for the
// Territory transfers tests. Nothing here comes from a game or a recording.
import type { PeaceChange, PeaceDeal, PeaceDealsReply, PeaceItem, PeaceTransferCountry, PeaceTransferGraph, PeaceTransferredState, PeaceUnobserved } from '../../generated/dashboard';

export const holder = (slot: number, tag: string, hour: number, countryId = 1000 + slot): PeaceTransferCountry => ({ slot, countryId, tag, hour });
export interface Move {
  state: number; from: PeaceTransferCountry; to: PeaceTransferCountry; before: number; after: number;
  vp?: string | null; vpHour?: number | null; released?: boolean; derived?: boolean;
}

/** The product's graph: one link per giver/taker holder pair, IDs naming both. */
export function graph(moves: readonly Move[]): PeaceTransferGraph {
  const nodes = new Map<string, PeaceTransferGraph['nodes'][number]>();
  const links = new Map<string, { id: string; source: string; target: string; states: PeaceTransferredState[] }>();
  for (const move of moves) {
    const source = `source:${move.from.slot}:${move.from.countryId}`, target = `target:${move.to.slot}:${move.to.countryId}`;
    for (const [id, country, side] of [[source, move.from, 'source'], [target, move.to, 'target']] as const) {
      const node = nodes.get(id);
      if (!node || country.hour > node.country.hour) nodes.set(id, { id, country, side });
    }
    const id = `${source.length}:${source}${target}`;
    const link = links.get(id) ?? { id, source, target, states: [] };
    const vp = move.vp === undefined ? '1' : move.vp;
    link.states.push({ state: move.state, fromCountry: move.from, toCountry: move.to, dates: { beforeHour: move.before, afterHour: move.after },
      victoryPoints: vp, victoryPointsReadHour: vp == null ? null : move.vpHour ?? move.after, released: move.released === true });
    links.set(id, link);
  }
  return { nodes: [...nodes.values()].sort((a, b) => a.id.localeCompare(b.id)), links: [...links.values()].map(link => {
    let sum: bigint | null = 0n;
    for (const state of link.states) sum = sum == null || state.victoryPoints == null ? null : sum + BigInt(state.victoryPoints);
    return { ...link, stateCount: link.states.length, victoryPoints: sum?.toString() ?? null };
  }) };
}

/** Plain moves become state-transfer items; released moves join one release item per taker. */
export function deal(id: string, moves: readonly Move[], extra: Partial<PeaceDeal> = {}): PeaceDeal {
  const items: PeaceItem[] = [];
  const releases = new Map<string, Move[]>();
  for (const move of moves) {
    const dates = { beforeHour: move.before, afterHour: move.after };
    const vp = move.vp === undefined ? '1' : move.vp;
    if (move.released) { const list = releases.get(move.to.tag) ?? []; list.push(move); releases.set(move.to.tag, list); continue; }
    items.push({ derived: move.derived ?? true, dates, winner: move.to.tag, action: { kind: 'stateTransfer', key: null, name: null },
      change: { kind: 'stateTransfer', state: move.state, from: move.from.tag, to: move.to.tag, fromCountry: move.from, toCountry: move.to,
        victoryPoints: vp, victoryPointsReadHour: vp == null ? null : move.vpHour ?? move.after } });
  }
  for (const [country, list] of releases) {
    const sorted = [...list].sort((a, b) => a.state - b.state);
    const change: PeaceChange = { kind: 'countryReleased', country, from: [...new Set(sorted.map(move => move.from.tag))].sort(), states: sorted.map(move => move.state),
      toCountry: sorted[0].to, transfers: sorted.map(move => {
        const vp = move.vp === undefined ? '1' : move.vp;
        return { state: move.state, fromCountry: move.from, victoryPoints: vp, victoryPointsReadHour: vp == null ? null : move.vpHour ?? move.after };
      }) };
    items.push({ derived: sorted[0].derived ?? true, dates: { beforeHour: sorted[0].before, afterHour: sorted[0].after }, winner: null,
      action: { kind: 'countryReleased', key: null, name: null }, change });
  }
  const hour = Math.max(...moves.map(move => move.after), extra.firstObservedHour ?? 0);
  return { id, derived: true, observation: 'recordedConference', conferenceId: id, warName: null, firstObservedHour: hour, lastObservedHour: hour,
    currentTurn: null, finalized: null, firstFinalizedHour: null, endHour: hour, endIsBy: true, firstAbsentHour: null, participantsDerived: true,
    winners: [], losers: [], items, captured: null, captureUnavailable: null, transfers: graph(moves), limitations: [], preparing: [], ...extra };
}

export const noGaps = (): PeaceUnobserved => ({ settlements: [], conferences: [], ownership: [], wars: [], subjects: [], governments: [], existence: [], victoryPoints: [] });
export function reply(deals: PeaceDeal[], unobserved: Partial<PeaceUnobserved> = {}): PeaceDealsReply {
  const moves = deals.flatMap(deal => deal.transfers.links.flatMap(link => link.states));
  const campaign = graph(moves.map(state => ({ state: state.state, from: state.fromCountry, to: state.toCountry, before: state.dates.beforeHour, after: state.dates.afterHour,
    vp: state.victoryPoints, vpHour: state.victoryPointsReadHour, released: state.released })));
  return { branchId: '1', derived: true, deals, transfers: campaign, unobserved: { ...noGaps(), ...unobserved }, warnings: [] };
}

/** The same reply from a bridge built before released states joined the graphs. */
export function olderBridge(current: PeaceDealsReply): PeaceDealsReply {
  const strip = (value: PeaceTransferGraph): PeaceTransferGraph => {
    const links = value.links.map(link => ({ ...link, states: link.states.filter(state => !state.released).map(({ released: _released, ...state }) => state as PeaceTransferredState) }))
      .filter(link => link.states.length).map(link => {
        let sum: bigint | null = 0n;
        for (const state of link.states) sum = sum == null || state.victoryPoints == null ? null : sum + BigInt(state.victoryPoints);
        return { ...link, stateCount: link.states.length, victoryPoints: sum?.toString() ?? null };
      });
    const used = new Set(links.flatMap(link => [link.source, link.target]));
    return { nodes: value.nodes.filter(node => used.has(node.id)), links };
  };
  return { ...current, transfers: strip(current.transfers), deals: current.deals.map(deal => ({ ...deal, transfers: strip(deal.transfers), items: deal.items.map(item => {
    if (item.change.kind !== 'countryReleased') return item;
    const { toCountry: _to, transfers: _transfers, ...change } = item.change;
    return { ...item, change: change as PeaceChange };
  }) })) };
}
