import { errorMessage } from './errors.ts';

export type CloseAbout = 'close' | 'install' | 'leave' | 'query';
export interface CloseAsk { id: number; about: CloseAbout }
export interface CloseBatch { asks: readonly CloseAsk[]; waiting: readonly number[]; holds: readonly CloseAsk[] }
export type LeavingFor = 'close' | 'update';

export interface CloseRequests {
  watch(): Promise<void>;
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

const unanswered = (about: CloseAbout, stays: boolean) =>
  about === 'close' ? stays ? UNKEPT : UNCLOSED : stays ? UNANSWERED_UNSAVED : UNANSWERED_SAVED;
const leavingFor = (about: CloseAbout): LeavingFor => about === 'close' ? 'close' : 'update';

export function closeGuard(host: CloseGuardHost, page: CloseGuardPage, report: (failures: readonly CloseGuardFailure[]) => void): CloseGuard {
  let ended = false, watching = false, watched = false, leavingByChoice = false;
  let shown: LeavingFor | null | undefined;
  const committed = new Map<number, LeavingFor>();
  const failures = new Map<string, CloseGuardFailure>();
  let live = new Set<number>();
  const publish = () => { if (!ended) report([...failures.values()]); };
  const clear = (key: string) => { if (failures.delete(key)) publish(); };
  const fail = (key: string, message: string, cause: unknown, again: () => void) => {
    const failure: CloseGuardFailure = { key, message, detail: errorMessage(cause), retry: () => { if (!ended && failures.get(key) === failure) again(); } };
    failures.set(key, failure);
    publish();
  };

  const leaving = () => leavingByChoice || committed.size > 0;
  const show = () => {
    const now = [...committed.values()].includes('update') ? 'update' : leaving() ? 'close' : null;
    if (ended || now === shown) return;
    shown = now;
    page.leaving(now);
  };
  const release = (id: number) => { if (committed.delete(id)) show(); };

  const send = (id: number, about: CloseAbout, stays: boolean, basis: readonly number[]) => {
    if (ended) return;
    const key = `answer:${id}`;
    clear(key);
    host.answer(id, stays, basis).then(heard => {
      if (ended) return;
      clear(key);
      if (!heard) release(id);
    }, cause => { if (!ended && live.has(id)) fail(key, unanswered(about, stays), cause, () => send(id, about, stays, basis)); });
  };

  const answer = (id: number, about: CloseAbout) => {
    if (ended) return;
    const unsaved = page.unsaved();
    let stays: boolean, basis: readonly number[] = [];
    if (about === 'query') stays = !leaving() && unsaved;
    else {
      stays = !leaving() && unsaved && !(about === 'install' && page.confirmed());
      if (!stays) {
        if (unsaved && !leavingByChoice) basis = [...committed.keys()];
        committed.set(id, leavingFor(about));
        show();
      } else if (about === 'close') page.prompt();
    }
    send(id, about, stays, basis);
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
    live = new Set([...batch.waiting, ...batch.holds.map(hold => hold.id)]);
    for (const id of [...committed.keys()]) if (!live.has(id)) committed.delete(id);
    const retired = [...failures.keys()].filter(key => key.startsWith('answer:') && !live.has(Number(key.slice('answer:'.length))));
    for (const key of retired) failures.delete(key);
    if (retired.length) publish();
    for (const hold of batch.holds) if (!committed.has(hold.id)) committed.set(hold.id, leavingFor(hold.about));
    show();
    for (const ask of batch.asks) answer(ask.id, ask.about);
    return true;
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
