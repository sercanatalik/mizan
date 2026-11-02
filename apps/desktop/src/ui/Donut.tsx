import { useState } from "react";
import s from "./chart.module.css";
import { slotColor } from "./palette";

export interface DonutSlice {
  name: string;
  value: number;
  /** Text shown in the legend, for example "37.9%". */
  label?: string;
}

/** A donut of shares with a 2px surface gap between slices, a legend and hover. */
export function Donut({ label, slices, size = 180 }: { label: string; slices: DonutSlice[]; size?: number }) {
  const [hover, setHover] = useState<number | null>(null);
  const total = slices.reduce((a, x) => a + Math.max(0, x.value), 0);
  const r = size / 2 - 4;
  const inner = r * 0.6;
  const c = size / 2;
  const fracs = slices.map((sl) => (total > 0 ? Math.max(0, sl.value) / total : 0));
  const starts = fracs.map((_, i) => fracs.slice(0, i).reduce((a, f) => a + f, 0));
  const arcs = slices.map((_, i) => {
    const frac = fracs[i]!;
    const a0 = -Math.PI / 2 + starts[i]! * Math.PI * 2;
    const a1 = a0 + frac * Math.PI * 2;
    const large = a1 - a0 > Math.PI ? 1 : 0;
    const p = (rad: number, a: number) => `${(c + rad * Math.cos(a)).toFixed(2)},${(c + rad * Math.sin(a)).toFixed(2)}`;
    if (frac >= 0.9999) {
      return `M${p(r, 0)}A${r},${r} 0 1 1 ${p(r, Math.PI)}A${r},${r} 0 1 1 ${p(r, 0)}ZM${p(inner, 0)}A${inner},${inner} 0 1 0 ${p(inner, Math.PI)}A${inner},${inner} 0 1 0 ${p(inner, 0)}Z`;
    }
    return `M${p(r, a0)}A${r},${r} 0 ${large} 1 ${p(r, a1)}L${p(inner, a1)}A${inner},${inner} 0 ${large} 0 ${p(inner, a0)}Z`;
  });
  const summary = `${label}: ${slices.map((x) => `${x.name} ${x.label ?? x.value}`).join(", ")}`;
  return (
    <figure className={s.figure} aria-label={label} style={{ display: "flex", alignItems: "center", gap: 16, flexWrap: "wrap" }}>
      <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size} role="img" aria-label={summary}>
        {arcs.map((d, i) => (
          <path
            key={i}
            d={d}
            fillRule="evenodd"
            style={{ fill: slotColor(i), opacity: hover === null || hover === i ? 1 : 0.45 }}
            stroke="var(--surface)"
            strokeWidth={2}
            onPointerEnter={() => setHover(i)}
            onPointerLeave={() => setHover(null)}
            data-testid={`slice-${i}`}
          />
        ))}
        {hover !== null && (
          <text x={c} y={c} textAnchor="middle" dominantBaseline="middle" className={s.tick} style={{ fontSize: 13 }}>
            {slices[hover]!.label ?? slices[hover]!.value}
          </text>
        )}
      </svg>
      <ul className={s.legend} aria-label={`${label} legend`} style={{ flexDirection: "column", alignItems: "flex-start" }}>
        {slices.map((sl, i) => (
          <li key={sl.name} onPointerEnter={() => setHover(i)} onPointerLeave={() => setHover(null)}>
            <span className={s.swatch} style={{ background: slotColor(i) }} aria-hidden="true" />
            {sl.name}
            {sl.label && <span style={{ color: "var(--text-muted)", marginLeft: 6 }}>{sl.label}</span>}
          </li>
        ))}
      </ul>
    </figure>
  );
}
