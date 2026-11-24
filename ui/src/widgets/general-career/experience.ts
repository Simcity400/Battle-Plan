import { formatScalar } from '../../families/valueFormat.ts';

export function experienceText(raw: string, required: string | null): string {
  const points = (text: string) => {
    const value = BigInt(text); const magnitude = value < 0n ? -value : value;
    return `${value < 0n ? '-' : ''}${magnitude / 100000n}.${(magnitude % 100000n).toString().padStart(5, '0').replace(/0+$/, '') || '0'}`;
  };
  return required === null ? points(raw) : `${points(raw)} / ${points(required)}`;
}

export function experienceWhole(raw: string, required: string | null): string {
  const whole = (text: string) => formatScalar(BigInt(text) / 100000n, 'integer');
  return required === null ? whole(raw) : `${whole(raw)} / ${whole(required)}`;
}

export function experienceFactorText(raw: string): string {
  const value = BigInt(raw);
  return `${value >= 0n ? '+' : ''}${experienceText((value * 100n).toString(), null)}%`;
}
