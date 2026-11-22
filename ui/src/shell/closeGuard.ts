import { errorMessage } from './errors.ts';

export type CloseAbout = 'close' | 'leave' | 'query';
export type CloseRequest = { kind: 'ask'; id: number; about: CloseAbout } | { kind: 'stay'; id: number };
export type LeavingFor = 'close' | 'update';

export interface CloseRequests {
  watch(): Promise<void>;
}

export interface CloseGuardHost {
  requests(onRequests: (requests: readonly CloseRequest[]) => void): CloseRequests;
  answer(id: number, stays: boolean): Promise<boolean>;
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

export function closeGuard(host: CloseGuardHost, page: CloseGuardPage, report: (failures: readonly CloseGuardFailure[]) => void): CloseGuard {
  let ended = false, watching = false, watched = false, leavingByChoice = false, shown: LeavingFor | null = null;
  const committed = new Map<number, LeavingFor>();
  const failures = new Map<string, CloseGuardFailure>();
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
    if (now === shown) return;
    shown = now;
    page.leaving(now);
  };
  const release = (id: number) => { if (committed.delete(id)) show(); };

  const answer = (id: number, about: CloseAbout) => {
    const key = `answer:${id}`;
    clear(key);
    let stays: boolean;
    if (about === 'query') stays = !leaving() && page.unsaved();
    else {
      stays = !leaving() && page.unsaved() && !(about === 'leave' && page.confirmed());
      if (!stays) {
        committed.set(id, about === 'leave' ? 'update' : 'close');
        show();
      } else if (about === 'close') page.prompt();
    }
    host.answer(id, stays).then(heard => {
      clear(key);
      if (!heard) release(id);
    }, cause => fail(key, unanswered(about, stays), cause, () => answer(id, about)));
  };

  const leave = () => {
    if (ended) return;
    clear('close');
    leavingByChoice = true;
    show();
    host.leave().catch(cause => fail('close', UNCLOSED, cause, leave));
  };

  const requests = host.requests(batch => {
    for (const request of batch) {
      if (ended) return;
      if (request.kind === 'ask') answer(request.id, request.about);
      else {
        clear(`answer:${request.id}`);
        release(request.id);
      }
    }
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
      fail('watch', UNHEARD, cause, watch);
    });
  };
  watch();
  return { leave, end() { ended = true; } };
}
