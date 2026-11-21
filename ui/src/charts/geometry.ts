import { flushSync } from 'react-dom';

type GeometryRead = () => (() => void);
const pending = new Set<GeometryRead>();
let queued = false;

export function invalidateGeometry(read: GeometryRead): void {
  pending.add(read);
  if (queued) return;
  queued = true;
  queueMicrotask(() => {
    queued = false;
    const reads = [...pending];
    pending.clear();
    const writes: (() => void)[] = [], errors: unknown[] = [];
    for (const read of reads) { try { writes.push(read()); } catch (error) { errors.push(error); } }
    flushSync(() => { for (const write of writes) { try { write(); } catch (error) { errors.push(error); } } });
    for (const error of errors) reportError(error);
  });
}

export function cancelGeometry(read: GeometryRead): void { pending.delete(read); }
