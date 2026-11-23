import { invoke } from '@tauri-apps/api/core';
import type { InvokeArgs } from '@tauri-apps/api/core';
import { enterWrite } from './pendingWrites.ts';

export async function tracked<T>(write: () => Promise<T>): Promise<T> {
  const pending = enterWrite();
  try {
    return await write();
  } finally {
    pending.saved();
  }
}

export const invokeWrite = <T>(command: string, args?: InvokeArgs): Promise<T> => tracked(() => invoke<T>(command, args));
