import { invoke } from '@tauri-apps/api/core';

export function afterShellPaint(run: () => void): () => void {
  let frame: number;
  const cancel = () => {
    cancelAnimationFrame(frame);
    document.removeEventListener('visibilitychange', visibility);
  };
  const ready = () => {
    cancel();
    run();
  };
  const visibility = () => { if (document.hidden) ready(); };
  if (document.hidden) {
    run();
    return () => {};
  }
  frame = requestAnimationFrame(() => { frame = requestAnimationFrame(ready); });
  document.addEventListener('visibilitychange', visibility);
  return cancel;
}

export function reportShellReady(): Promise<void> {
  return invoke('shell_ready');
}
