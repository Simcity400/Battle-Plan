export interface Credits { owe(bytes: number): void; owed(): number }

export function credits(send: (bytes: number) => Promise<boolean>, later: (run: () => void) => void): Credits {
  let owed = 0, sending = false, due = false;
  const schedule = () => { if (!due) { due = true; later(() => { void flush(); }); } };
  const flush = async () => {
    due = false;
    if (sending || !owed) return;
    const bytes = owed;
    owed = 0; sending = true;
    let sent = false;
    try { sent = await send(bytes); } catch { sent = false; }
    sending = false;
    if (!sent) owed += bytes;
    if (owed) schedule();
  };
  return { owe(bytes) { if (bytes > 0) { owed += bytes; schedule(); } }, owed: () => owed };
}
