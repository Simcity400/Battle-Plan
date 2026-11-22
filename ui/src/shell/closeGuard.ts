import { errorMessage } from './errors.ts';

export type CloseRequest = { kind: 'close' } | { kind: 'ask'; id: number };

export interface CloseRequests {
  watch(): Promise<void>;
}

export interface CloseGuardHost {
  requests(onRequest: (request: CloseRequest) => void): CloseRequests;
  answer(id: number, unsaved: boolean): Promise<void>;
  leave(): Promise<void>;
}

export interface CloseGuardPage {
  unsaved(): boolean;
  ask(): void;
}

export interface CloseGuardFailure { key: string; message: string; detail: string; retry(): void }

export interface CloseGuard {
  end(): void;
}

export const UNHEARD = 'Couldn’t watch for BattlePlan closing, so closing it won’t ask before discarding unsaved changes.';
export const UNCLOSED = 'Couldn’t close BattlePlan.';
export const UNANSWERED_SAVED = 'Couldn’t tell BattlePlan your changes are saved, so its update waits.';
export const UNANSWERED_UNSAVED = 'Couldn’t tell BattlePlan you have unsaved changes, so its update waits.';

export function closeGuard(host: CloseGuardHost, page: CloseGuardPage, report: (failures: readonly CloseGuardFailure[]) => void): CloseGuard {
  let ended = false, watching = false, watched = false;
  const failures = new Map<string, CloseGuardFailure>();
  const publish = () => { if (!ended) report([...failures.values()]); };
  const clear = (key: string) => { if (failures.delete(key)) publish(); };
  const fail = (key: string, message: string, cause: unknown, again: () => void) => {
    const failure: CloseGuardFailure = { key, message, detail: errorMessage(cause), retry: () => { if (!ended && failures.get(key) === failure) again(); } };
    failures.set(key, failure);
    publish();
  };

  const close = () => {
    clear('close');
    if (page.unsaved()) { page.ask(); return; }
    host.leave().catch(cause => fail('close', UNCLOSED, cause, close));
  };
  const answer = (id: number) => {
    const key = `ask:${id}`;
    const unsaved = page.unsaved();
    host.answer(id, unsaved).then(() => clear(key),
      cause => fail(key, unsaved ? UNANSWERED_UNSAVED : UNANSWERED_SAVED, cause, () => answer(id)));
  };
  const requests = host.requests(request => {
    if (ended) return;
    if (request.kind === 'ask') answer(request.id);
    else close();
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
  return { end() { ended = true; } };
}
