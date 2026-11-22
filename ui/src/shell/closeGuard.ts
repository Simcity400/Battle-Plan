import { errorMessage } from './errors.ts';

export interface CloseGuardHost {
  mark(unsaved: boolean): Promise<void>;
  listen(onClose: () => void): Promise<() => void>;
  close(): Promise<void>;
}

export interface CloseGuardFailure { message: string; detail: string }

export interface CloseGuard {
  mark(unsaved: boolean): void;
  retry(): void;
  end(): void;
}

export const UNGUARDED = 'Couldn’t guard your unsaved changes, so closing BattlePlan won’t ask before discarding them.';
export const UNCLEARED = 'Couldn’t tell BattlePlan your changes are saved, so it won’t restart to update yet.';
export const UNHEARD = 'Couldn’t watch for BattlePlan closing, so it won’t close while changes are unsaved.';
export const UNCLOSED = 'Couldn’t close BattlePlan.';

export function closeGuard(host: CloseGuardHost, onClose: () => void, report: (failure: CloseGuardFailure | null) => void): CloseGuard {
  let wanted: boolean | null = null, held: boolean | null = null;
  let marking = false, listening = false, ended = false;
  let unmarked: CloseGuardFailure | null = null, unheard: CloseGuardFailure | null = null, unclosed: CloseGuardFailure | null = null;
  let stop: (() => void) | null = null;
  let shown: CloseGuardFailure | null = null;
  const publish = () => {
    const failure = unmarked ?? unheard ?? unclosed;
    if (ended || failure === shown) return;
    shown = failure;
    report(failure);
  };
  const send = () => {
    if (marking || ended || wanted === null) return;
    marking = true;
    const unsaved = wanted;
    host.mark(unsaved).then(() => {
      held = unsaved;
      unmarked = null;
    }, cause => {
      held = null;
      unmarked = { message: unsaved ? UNGUARDED : UNCLEARED, detail: errorMessage(cause) };
    }).finally(() => {
      marking = false;
      if (wanted !== unsaved) send();
      else publish();
    });
  };
  const requested = () => {
    if (ended) return;
    unclosed = null;
    if (wanted) {
      publish();
      onClose();
      return;
    }
    host.close().catch(cause => {
      unclosed = { message: UNCLOSED, detail: errorMessage(cause) };
    }).finally(publish);
  };
  const listen = () => {
    if (listening || stop || ended) return;
    listening = true;
    host.listen(requested).then(unlisten => {
      if (ended) { unlisten(); return; }
      stop = unlisten;
      unheard = null;
    }, cause => {
      unheard = { message: UNHEARD, detail: errorMessage(cause) };
    }).finally(() => {
      listening = false;
      publish();
    });
  };
  listen();
  return {
    mark(unsaved) {
      wanted = unsaved;
      if (unsaved !== held || unmarked) send();
    },
    retry() {
      if (unmarked) send();
      if (unheard) listen();
      if (unclosed) requested();
    },
    end() {
      ended = true;
      stop?.();
      stop = null;
    },
  };
}
