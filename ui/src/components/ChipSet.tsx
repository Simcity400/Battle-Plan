import './ChipSet.css';

export interface Chip { value: string; label: string; icon?: string | null; colour?: string; unavailable?: string; hint?: string }

export function ChipSet({ label, chips, selected, max, full: atMax, onToggle }: {
  label: string; chips: readonly Chip[]; selected: readonly string[]; max: number; full?: boolean; onToggle: (value: string) => void;
}) {
  const full = atMax ?? selected.length >= max;
  return <div className="chip-set" role="group" aria-label={label}>{chips.map(chip => {
    const pressed = selected.includes(chip.value);
    const reason = chip.unavailable ?? (!pressed && full ? `At most ${max} can be chosen` : undefined);
    return <button key={chip.value} type="button" className="chip" aria-pressed={pressed} aria-disabled={reason ? true : undefined}
      data-tooltip={[chip.hint, reason].filter(Boolean).join('\n') || undefined} onClick={() => { if (!reason) onToggle(chip.value); }}>
      {chip.colour && <i className="chip-dot" style={{ background: chip.colour }} aria-hidden="true" />}
      {chip.icon && <img src={chip.icon} alt="" />}{chip.label}
    </button>;
  })}</div>;
}
