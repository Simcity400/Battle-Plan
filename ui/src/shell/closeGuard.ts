import { errorMessage } from './errors.ts';

export type CloseAbout = 'close' | 'install' | 'leave' | 'query';
export interface CloseAsk { id: number; about: CloseAbout }
export interface CloseBatch { seq: number; waiting: readonly CloseAsk[]; holds: readonly CloseAsk[]; installing: boolean }
export type LeavingFor = 'close' | 'update';

export interface CloseRequests {
  watch(): Promise<void>;
  refresh(givingUpRestarts: boolean): Promise<number>;
}

export interface CloseGuardHost {
  requests(onBatch: (batch: CloseBatch) => boolean): CloseRequests;
  answer(id: number, stays: boolean, basis: readonly number[]): Promise<boolean>;
  leave(): Promise<void>;
}

export interface CloseGuardPage {
  unsaved(): boolean;
  confirmed(): boolean;
  prompt(): void;
  leaving(leaving: LeavingFor | null): void;
}

export interface CloseGuardFailure { key: string; message: string; detail: string; retry(): void }

export interface CloseGuard {
  leave(): void;
  recheck(): void;
  end(): void;
}

export const UNHEARD = 'Couldn’t watch for BattlePlan closing, so it can’t check for unsaved changes. Closing waits; close again to choose whether to close without checking. Updates wait too.';
export const UNCLOSED = 'Couldn’t close BattlePlan.';
export const UNKEPT = 'Couldn’t tell BattlePlan to stay open for your unsaved changes.';
export const UNANSWERED_SAVED = 'Couldn’t tell BattlePlan your changes are saved, so its update waits.';
export const UNANSWERED_UNSAVED = 'Couldn’t tell BattlePlan you have unsaved changes, so its update waits.';
export const UNJUDGED = 'Couldn’t check for unsaved changes, so BattlePlan’s close and update questions wait.';
export const UNRECHECKED = 'The update didn’t start, and BattlePlan hasn’t said whether anything still holds this window, so it stays locked.';
export const AWAITED = 'BattlePlan hasn’t yet told this window what stands.';

const unanswered = (about: CloseAbout, stays: boolean) =>
  about === 'close' ? stays ? UNKEPT : UNCLOSED : stays ? UNANSWERED_UNSAVED : UNANSWERED_SAVED;
const leavingFor = (about: CloseAbout): LeavingFor => about === 'close' ? 'close' : 'update';

export function closeGuard(host: CloseGuardHost, page: CloseGuardPage, report: (failures: readonly CloseGuardFailure[]) => void): CloseGuard {
  let ended = false, watching = false, watched = false, leavingByChoice = false, unresolved = false, installing = false, applied = -1;
  let recheckAfter: number | undefined;
  let shown: LeavingFor | null | undefined;
  const committed = new Map<number, { leaving: LeavingFor; accepted: boolean }>();
  const judged = new Set<number>();
  const resend = new Map<number, { after: number; failure: CloseGuardFailure | undefined; again: () => void }>();
  let waiting: readonly CloseAsk[] = [];
  const failures = new Map<string, CloseGuardFailure>();
  const refreshing = new Set<CloseGuardFailure>();
  const asked = (id: number) => waiting.some(ask => ask.id === id);
  const publish = () => { if (!ended) report([...failures.values()]); };
  const clear = (key: string) => { if (failures.delete(key)) publish(); };
  const fail = (key: string, message: string, cause: unknown, again: () => void) => {
    const failure: CloseGuardFailure = { key, message, detail: errorMessage(cause), retry: () => { if (!ended && failures.get(key) === failure) again(); } };
    failures.set(key, failure);
    publish();
  };

  const refresh = (key: string, message: string, again: () => void, fresh?: (seq: number, failure: CloseGuardFailure | undefined) => void, keep = () => true) => {
    const failure = failures.get(key);
    if (failure && refreshing.has(failure)) return;
    if (failure) refreshing.add(failure);
    const current = () => { if (failure) refreshing.delete(failure); return !ended && failures.get(key) === failure; };
    requests.refresh(key === 'recheck').then(seq => { if (current()) fresh?.(seq, failure); }, cause => { if (current() && keep()) fail(key, message, cause, again); });
  };
  const resendDue = () => {
    for (const [id, { after, failure, again }] of [...resend]) {
      if (after > applied) continue;
      resend.delete(id);
      if (failures.get(`answer:${id}`) === failure && asked(id)) again();
    }
  };
  const rejudge = () => refresh('judge', UNJUDGED, rejudge);
  const updating = () => [...committed.values()].some(commitment => commitment.leaving === 'update');
  const recheck = () => {
    if (!failures.has('recheck') && committed.size > 0) fail('recheck', UNRECHECKED, AWAITED, recheck);
    refresh('recheck', UNRECHECKED, recheck, seq => {
      if (applied < seq || updating() || installing) {
        recheckAfter = seq;
        if (committed.size > 0) fail('recheck', UNRECHECKED, AWAITED, recheck);
        return;
      }
      recheckAfter = undefined;
      unresolved = false;
      clear('recheck');
    }, () => committed.size > 0);
  };

  const leaving = () => leavingByChoice || committed.size > 0;
  const show = () => {
    const now = [...committed.values()].some(commitment => commitment.leaving === 'update') ? 'update' : leaving() ? 'close' : null;
    if (!ended && shown === 'update' && now !== 'update') unresolved = false;
    if (committed.size === 0) { if (!unresolved) recheckAfter = undefined; clear('recheck'); }
    if (ended || now === shown) return;
    shown = now;
    page.leaving(now);
    if (now === 'update' && unresolved && !failures.has('recheck')) recheck();
  };

  const send = (id: number, about: CloseAbout, stays: boolean, basis: readonly number[]) => {
    if (ended) return;
    const key = `answer:${id}`;
    clear(key);
    host.answer(id, stays, basis).then(heard => {
      if (ended) return;
      clear(key);
      const commitment = committed.get(id);
      if (!commitment) return;
      if (heard) commitment.accepted = true;
      else committed.delete(id);
      show();
      if (!failures.has('judge')) judge(waiting, true);
    }, cause => {
      if (ended || !asked(id)) return;
      const message = unanswered(about, stays);
      const again = () => refresh(key, message, again, (after, failure) => {
        resend.set(id, { after, failure, again: () => send(id, about, stays, basis) });
        resendDue();
      });
      fail(key, message, cause, again);
    });
  };

  const judge = (asks: readonly CloseAsk[], whole: boolean): boolean => {
    for (const { id, about } of asks) {
      if (ended) return false;
      if (judged.has(id)) continue;
      let unsaved: boolean, confirmed: boolean;
      try {
        unsaved = page.unsaved();
        confirmed = about === 'install' && page.confirmed();
      } catch (cause) {
        fail('judge', UNJUDGED, cause, rejudge);
        return false;
      }
      const now = leaving();
      if (about === 'query') {
        judged.add(id);
        send(id, about, !now && unsaved, []);
        continue;
      }
      if (!now && unsaved && !confirmed) {
        judged.add(id);
        if (about === 'close') page.prompt();
        send(id, about, true, []);
        continue;
      }
      const leaning = unsaved && !leavingByChoice && !confirmed;
      if (leaning && [...committed.values()].some(commitment => !commitment.accepted)) continue;
      const basis = leaning ? [...committed.keys()] : [];
      judged.add(id);
      committed.set(id, { leaving: leavingFor(about), accepted: false });
      show();
      send(id, about, false, basis);
    }
    if (whole || waiting.every(({ id }) => judged.has(id))) clear('judge');
    return true;
  };

  const leave = () => {
    if (ended) return;
    clear('close');
    leavingByChoice = true;
    show();
    host.leave().catch(cause => { if (!ended) fail('close', UNCLOSED, cause, leave); });
  };

  const requests = host.requests(batch => {
    if (ended) return false;
    if (batch.seq <= applied) return judge(batch.waiting.filter(ask => waiting.some(({ id }) => id === ask.id)), false);
    applied = batch.seq;
    waiting = batch.waiting;
    installing = batch.installing;
    const live = new Set([...batch.waiting, ...batch.holds].map(ask => ask.id));
    for (const id of [...committed.keys()]) if (!live.has(id)) committed.delete(id);
    for (const id of [...judged]) if (!live.has(id)) judged.delete(id);
    const rechecked = recheckAfter !== undefined && batch.seq >= recheckAfter;
    const settled = [...failures.keys()].filter(key => key.startsWith('answer:') && !asked(Number(key.slice('answer:'.length))));
    for (const hold of batch.holds) {
      const commitment = committed.get(hold.id);
      if (commitment) commitment.accepted = true;
      else committed.set(hold.id, { leaving: leavingFor(hold.about), accepted: true });
    }
    for (const key of settled) failures.delete(key);
    if (settled.length) publish();
    show();
    resendDue();
    const handled = judge(waiting, true);
    if (!ended && rechecked && !updating() && !installing) {
      recheckAfter = undefined;
      unresolved = false;
      clear('recheck');
    }
    return handled;
  });
  const watch = () => {
    if (ended || watching || watched) return;
    watching = true;
    requests.watch().then(() => {
      watching = false;
      watched = true;
      clear('watch');
    }, cause => {
      watching = false;
      if (!ended) fail('watch', UNHEARD, cause, watch);
    });
  };
  watch();
  return { leave, recheck() { if (!ended) { unresolved = true; recheck(); } }, end() { ended = true; } };
}
