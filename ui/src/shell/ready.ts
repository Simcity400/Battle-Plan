import { invoke } from '@tauri-apps/api/core';

export function afterShellPaint(run: () => void): () => void {
  let frame = 0;
  let pending = true;
  const cancel = () => {
    pending = false;
    cancelAnimationFrame(frame);
    document.removeEventListener('visibilitychange', visibility);
  };
  const ready = () => {
    if (!pending) return;
    cancel();
    run();
  };
  const visibility = () => { if (document.hidden) ready(); };
  if (document.hidden) {
    queueMicrotask(ready);
  } else {
    frame = requestAnimationFrame(() => { frame = requestAnimationFrame(ready); });
    document.addEventListener('visibilitychange', visibility);
  }
  return cancel;
}

export function reportShellReady(): Promise<void> {
  return invoke('shell_ready');
}
