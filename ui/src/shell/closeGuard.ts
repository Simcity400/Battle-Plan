import { errorMessage } from './errors.ts';

export type CloseAbout = 'close' | 'install' | 'leave' | 'query';
export interface CloseAsk { id: number; about: CloseAbout }
export interface CloseBatch { seq: number; waiting: readonly CloseAsk[]; holds: readonly CloseAsk[] }
export type LeavingFor = 'close' | 'update';

export interface CloseRequests {
  watch(): Promise<void>;
  refresh(): Promise<void>;
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
  end(): void;
}

export const UNHEARD = 'Couldn’t watch for BattlePlan closing, so it can’t check for unsaved changes. Closing waits; close again to choose whether to close without checking. Updates wait too.';
export const UNCLOSED = 'Couldn’t close BattlePlan.';
export const UNKEPT = 'Couldn’t tell BattlePlan to stay open for your unsaved changes.';
export const UNANSWERED_SAVED = 'Couldn’t tell BattlePlan your changes are saved, so its update waits.';
export const UNANSWERED_UNSAVED = 'Couldn’t tell BattlePlan you have unsaved changes, so its update waits.';
export const UNJUDGED = 'Couldn’t check for unsaved changes, so BattlePlan’s close and update questions wait.';

const unanswered = (about: CloseAbout, stays: boolean) =>
  about === 'close' ? stays ? UNKEPT : UNCLOSED : stays ? UNANSWERED_UNSAVED : UNANSWERED_SAVED;
const leavingFor = (about: CloseAbout): LeavingFor => about === 'close' ? 'close' : 'update';

export function closeGuard(host: CloseGuardHost, page: CloseGuardPage, report: (failures: readonly CloseGuardFailure[]) => void): CloseGuard {
  let ended = false, watching = false, watched = false, leavingByChoice = false, applied = -1;
  let shown: LeavingFor | null | undefined;
  const committed = new Map<number, { leaving: LeavingFor; accepted: boolean }>();
  const judged = new Set<number>();
  const resend = new Map<number, () => void>();
  let waiting: readonly CloseAsk[] = [];
  const failures = new Map<string, CloseGuardFailure>();
  let live = new Set<number>();
  const publish = () => { if (!ended) report([...failures.values()]); };
  const clear = (key: string) => { if (failures.delete(key)) publish(); };
  const fail = (key: string, message: string, cause: unknown, again: () => void) => {
    const failure: CloseGuardFailure = { key, message, detail: errorMessage(cause), retry: () => { if (!ended && failures.get(key) === failure) again(); } };
    failures.set(key, failure);
    publish();
  };

  const refresh = (key: string, message: string, again: () => void) => {
    requests.refresh().catch(cause => { if (!ended && failures.has(key)) fail(key, message, cause, again); });
  };
  const rejudge = () => refresh('judge', UNJUDGED, rejudge);

  const leaving = () => leavingByChoice || committed.size > 0;
  const show = () => {
    const now = [...committed.values()].some(commitment => commitment.leaving === 'update') ? 'update' : leaving() ? 'close' : null;
    if (ended || now === shown) return;
    shown = now;
    page.leaving(now);
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
      if (ended || !live.has(id)) return;
      const message = unanswered(about, stays);
      const again = () => {
        resend.set(id, () => send(id, about, stays, basis));
        refresh(key, message, again);
      };
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
    live = new Set([...batch.waiting, ...batch.holds].map(ask => ask.id));
    for (const id of [...committed.keys()]) if (!live.has(id)) committed.delete(id);
    for (const id of [...judged]) if (!live.has(id)) judged.delete(id);
    const retired = [...failures.keys()].filter(key => key.startsWith('answer:') && !live.has(Number(key.slice('answer:'.length))));
    for (const key of retired) failures.delete(key);
    if (retired.length) publish();
    for (const hold of batch.holds) {
      const commitment = committed.get(hold.id);
      if (commitment) commitment.accepted = true;
      else committed.set(hold.id, { leaving: leavingFor(hold.about), accepted: true });
    }
    waiting = batch.waiting;
    show();
    const resent = [...resend].filter(([id]) => failures.has(`answer:${id}`));
    resend.clear();
    for (const [, again] of resent) again();
    return judge(waiting, true);
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
  return { leave, end() { ended = true; } };
}
