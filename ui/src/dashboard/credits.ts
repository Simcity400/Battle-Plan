export interface Credits { owe(bytes: number): void; owed(): number }

export function credits(send: (released: number) => Promise<boolean>, later: (run: () => void) => void): Credits {
  let released = 0, acknowledged = 0, sending = false, due = false;
  const schedule = () => { if (!due) { due = true; later(() => { void flush(); }); } };
  const flush = async () => {
    due = false;
    if (sending || released === acknowledged) return;
    const total = released;
    sending = true;
    let sent = false;
    try { sent = await send(total); } catch { sent = false; }
    sending = false;
    if (sent) acknowledged = Math.max(acknowledged, total);
    if (released !== acknowledged) schedule();
  };
  return { owe(bytes) { if (bytes > 0) { released += bytes; schedule(); } }, owed: () => released - acknowledged };
}
