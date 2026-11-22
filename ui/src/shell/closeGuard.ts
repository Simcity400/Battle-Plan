import { errorMessage } from './errors.ts';

export interface CloseGuardHost {
  mark(unsaved: boolean): Promise<void>;
  listen(onClose: () => void): Promise<() => void>;
}

export interface CloseGuardFailure { message: string; detail: string }

export interface CloseGuard {
  mark(unsaved: boolean): void;
  retry(): void;
  end(): void;
}

export const UNGUARDED = 'Couldn’t guard your unsaved changes, so closing BattlePlan won’t ask before discarding them.';
export const UNCLEARED = 'Couldn’t tell BattlePlan your changes are saved, so it won’t close or restart to update yet.';
export const UNHEARD = 'Couldn’t watch for BattlePlan closing, so it won’t close while changes are unsaved.';

export function closeGuard(host: CloseGuardHost, onClose: () => void, report: (failure: CloseGuardFailure | null) => void): CloseGuard {
  let wanted: boolean | null = null, sent = 0, ended = false;
  let marking: CloseGuardFailure | null = null, listening: CloseGuardFailure | null = null;
  let stop: (() => void) | null = null;
  let shown: CloseGuardFailure | null = null;
  const publish = () => {
    const failure = marking ?? listening;
    if (ended || failure === shown) return;
    shown = failure;
    report(failure);
  };
  const mark = (unsaved: boolean) => {
    wanted = unsaved;
    const ticket = ++sent;
    host.mark(unsaved).then(() => {
      if (ticket === sent) marking = null;
    }, cause => {
      if (ticket === sent) marking = { message: unsaved ? UNGUARDED : UNCLEARED, detail: errorMessage(cause) };
    }).finally(publish);
  };
  const listen = () => {
    host.listen(onClose).then(unlisten => {
      if (ended) { unlisten(); return; }
      stop = unlisten;
      listening = null;
    }, cause => {
      listening = { message: UNHEARD, detail: errorMessage(cause) };
    }).finally(publish);
  };
  listen();
  return {
    mark,
    retry() {
      if (marking && wanted !== null) mark(wanted);
      if (listening) listen();
    },
    end() {
      ended = true;
      stop?.();
    },
  };
}
