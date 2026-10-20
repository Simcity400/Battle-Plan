import type { DiplomacyPact, DiplomacyProposal, DiplomacyReadRun } from '../../generated/dashboard';

type Pact = DiplomacyPact;
type Proposal = DiplomacyProposal;

export type PactState = { state: 'present' } | { state: 'gone'; by: number } | { state: 'unknown'; since: number };

/** Each owner's supplying reads, in hour order, for dating when a listed pact stopped being listed. */
export function ownerReads(reads: readonly DiplomacyReadRun[]): Map<string, number[]> {
  const byOwner = new Map<string, number[]>();
  for (const read of reads) {
    if (!read.owner) continue;
    const key = `${read.owner.slot}:${read.owner.countryId}`;
    const hours = byOwner.get(key) ?? byOwner.set(key, []).get(key)!;
    if (hours.at(-1) !== read.readHour) hours.push(read.readHour);
  }
  for (const hours of byOwner.values()) hours.sort((a, b) => a - b);
  return byOwner;
}

/** Whether the recording lists a pact at `at`: listed through its last read (or still at the
 * last read); gone once a later read of its owner no longer lists it, dated by that read; and
 * unknown in between or where its owner was not read again, never assumed gone. Nothing is
 * said about the game's own state words. */
export function pactState(pact: Pact, at: number, reads: ReadonlyMap<string, readonly number[]>): PactState {
  if (at <= pact.lastSeen || pact.presentAtEnd) return { state: 'present' };
  // An episode a recording gap ended proves no absence: the next read may list it again.
  if (pact.endedBy) return { state: 'unknown', since: pact.lastSeen };
  const owner = pact.identities[0];
  const next = owner ? reads.get(`${owner.slot}:${owner.countryId}`)?.find(hour => hour > pact.lastSeen) : undefined;
  return next !== undefined && next <= at ? { state: 'gone', by: next } : { state: 'unknown', since: pact.lastSeen };
}

/** Pacts revealed by `at` (a later observation never shows early), narrowed by kind; Ended
 * hidden keeps only those still listed. */
export function shownPacts(pacts: readonly Pact[], at: number, kinds: ReadonlySet<string> | null, ended: boolean, reads: ReadonlyMap<string, readonly number[]>): Pact[] {
  return pacts.filter(pact => pact.firstSeen <= at && (!kinds || kinds.has(pact.kind)) && (ended || pactState(pact, at, reads).state === 'present'));
}

/** Where a pact's presence starts: its stored start, else the first read that listed it (a bound). */
export const pactSince = (pact: Pact) => pact.startHour ?? pact.firstSeen;

/** Whole days between two hours. */
export const days = (from: number, to: number) => Math.max(0, Math.floor((to - from) / 24));
export const daysText = (count: number) => `${count} ${count === 1 ? 'day' : 'days'}`;

export type ProposalState = 'waiting' | 'gone' | 'unknown';
/** Waiting through the episode's last read listing it (or still at the last read); gone once
 * a read no longer lists it; unknown between those reads or where reads stopped, so an
 * episode a gap suspended never counts as waiting. */
export function proposalState(proposal: Proposal, at: number): ProposalState {
  if (at <= proposal.lastSeen || proposal.presentAtEnd) return 'waiting';
  return proposal.goneBy != null && proposal.goneBy <= at ? 'gone' : 'unknown';
}

/** Proposals revealed by `at`, newest stored first; Waiting only keeps those still queued then. */
export function shownProposals(proposals: readonly Proposal[], at: number, waitingOnly: boolean): Proposal[] {
  return proposals.filter(item => item.firstSeen <= at && (!waitingOnly || proposalState(item, at) === 'waiting'));
}

/** The hour a card is dated by: its stored date, else when it was first seen. */
export const proposalHour = (proposal: Proposal) => proposal.storedHour ?? proposal.firstSeen;

/** Column tracks: names keep a few letters and ellipsise (full names in their titles), so the
 * table fits a small card once Since and Status hide. */
export const PACT_WIDTHS = { kind: 'minmax(6em, 1.2fr)', country: 'minmax(5em, 1fr)' } as const;
