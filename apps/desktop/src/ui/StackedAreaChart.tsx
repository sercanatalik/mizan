import { useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import s from "./chart.module.css";
import { slotColor } from "./palette";

export interface StackedSeries {
  name: string;
  /** One value per date; `null` means the series has no value that day (drawn as zero). */
  values: (number | null)[];
}

export interface StackedAreaChartProps {
  label: string;
  dates: string[];
  /** Bottom band first. At most 8; fold the rest into "Other" before passing them. */
  series: StackedSeries[];
  format: (v: number) => string;
  formatDate?: (d: string) => string;
  height?: number;
}

const W = 720;
const PAD = { top: 12, right: 12, bottom: 26, left: 72 };

function niceStep(range: number, count: number): number {
  const raw = range / count || 1;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const n = raw / mag;
  return (n <= 1 ? 1 : n <= 2 ? 2 : n <= 2.5 ? 2.5 : n <= 5 ? 5 : 10) * mag;
}

/** Stacked bands, each separated by a 2px surface gap, with a crosshair tooltip and table view. */
export function StackedAreaChart({ label, dates, series, format, formatDate = (d) => d, height = 260 }: StackedAreaChartProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const n = dates.length;

  const geom = useMemo(() => {
    const tops: number[][] = [];
    const run = new Array<number>(n).fill(0);
    for (const ser of series) {
      tops.push(ser.values.map((v, i) => (run[i] = run[i]! + Math.max(0, v ?? 0))));
    }
    const hi = Math.max(1, ...run);
    const step = niceStep(hi, 4);
    const y1 = Math.ceil(hi / step) * step;
    const ticks: number[] = [];
    for (let v = 0; v <= y1 + step / 2; v += step) ticks.push(Number(v.toFixed(10)));
    const plotW = W - PAD.left - PAD.right;
    const plotH = height - PAD.top - PAD.bottom;
    const x = (i: number) => PAD.left + (n <= 1 ? plotW / 2 : (i / (n - 1)) * plotW);
    const y = (v: number) => PAD.top + plotH - (v / (y1 || 1)) * plotH;
    const areas = tops.map((top, si) => {
      const bottom = si === 0 ? new Array<number>(n).fill(0) : tops[si - 1]!;
      if (n === 0) return "";
      const up = top.map((v, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join("");
      const down = bottom
        .map((v, i) => [i, v] as const)
        .reverse()
        .map(([i, v]) => `L${x(i).toFixed(1)},${y(v).toFixed(1)}`)
        .join("");
      return `${up}${down}Z`;
    });
    const edges = tops.map((top) => top.map((v, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(""));
    return { ticks, x, y, plotH, areas, edges, totals: run };
  }, [series, n, height]);

  const xLabels = n <= 1 ? [0] : [0, Math.round((n - 1) / 3), Math.round((2 * (n - 1)) / 3), n - 1];
  const pick = (e: PointerEvent<SVGSVGElement>) => {
    const r = svgRef.current?.getBoundingClientRect();
    if (!r || n === 0) return;
    const px = ((e.clientX - r.left) / r.width) * W;
    const i = Math.round(((px - PAD.left) / (W - PAD.left - PAD.right)) * (n - 1));
    setHover(Math.max(0, Math.min(n - 1, i)));
  };
  const onKey = (e: KeyboardEvent<SVGSVGElement>) => {
    if (e.key === "ArrowRight") setHover((h) => Math.min(n - 1, (h ?? -1) + 1));
    if (e.key === "ArrowLeft") setHover((h) => Math.max(0, (h ?? n) - 1));
  };
  const last = n - 1;
  const summary = `${label}: ${series.length} series, ${n ? `${formatDate(dates[0]!)} to ${formatDate(dates[last]!)}` : "no data"}, total ${n ? format(geom.totals[last]!) : "none"} at the end.`;
  const tableRows = n <= 60 ? [...Array(n).keys()].reverse() : [...Array(60).keys()].map((k) => Math.round((k * last) / 59)).reverse();

  return (
    <figure className={s.figure} aria-label={label}>
      <ul className={s.legend} aria-label={`${label} legend`}>
        {series.map((ser, si) => (
          <li key={ser.name}>
            <span className={s.swatch} style={{ background: slotColor(si) }} aria-hidden="true" />
            {ser.name}
          </li>
        ))}
      </ul>
      <div className={s.plot}>
        <svg
          ref={svgRef}
          viewBox={`0 0 ${W} ${height}`}
          role="img"
          aria-label={summary}
          tabIndex={0}
          className={s.svg}
          onPointerMove={pick}
          onPointerLeave={() => setHover(null)}
          onKeyDown={onKey}
          onBlur={() => setHover(null)}
        >
          {geom.ticks.map((t) => (
            <g key={t}>
              <line x1={PAD.left} x2={W - PAD.right} y1={geom.y(t)} y2={geom.y(t)} className={s.grid} />
              <text x={PAD.left - 8} y={geom.y(t)} className={s.tick} textAnchor="end" dominantBaseline="middle">
                {format(t)}
              </text>
            </g>
          ))}
          {xLabels.map((i, k) => (
            <text key={k} x={geom.x(i)} y={height - 6} className={s.tick} textAnchor={k === 0 ? "start" : k === xLabels.length - 1 ? "end" : "middle"}>
              {n ? formatDate(dates[i]!) : ""}
            </text>
          ))}
          {geom.areas.map((d, si) => (
            <path key={si} d={d} style={{ fill: slotColor(si), fillOpacity: 0.85 }} data-testid={`band-${si}`} />
          ))}
          {geom.edges.map((d, si) => (
            <path key={`e${si}`} d={d} fill="none" stroke="var(--surface)" strokeWidth={2} vectorEffect="non-scaling-stroke" />
          ))}
          {hover !== null && (
            <line x1={geom.x(hover)} x2={geom.x(hover)} y1={PAD.top} y2={PAD.top + geom.plotH} className={s.crosshair} aria-hidden="true" />
          )}
        </svg>
        {hover !== null && (
          <div
            role="status"
            className={s.tooltip}
            style={{
              left: `${(geom.x(hover) / W) * 100}%`,
              transform: geom.x(hover) > W * 0.6 ? "translateX(calc(-100% - 12px))" : "translateX(12px)",
            }}
          >
            <div className={s.tipDate}>{formatDate(dates[hover]!)}</div>
            {[...series].reverse().map((ser) => {
              const si = series.indexOf(ser);
              const v = ser.values[hover];
              return (
                <div key={ser.name} className={s.tipRow}>
                  <span className={s.swatch} style={{ background: slotColor(si) }} aria-hidden="true" />
                  <span>{ser.name}</span>
                  <span className={s.tipValue}>{v === null || v === undefined ? "—" : format(v)}</span>
                </div>
              );
            })}
            <div className={s.tipRow}>
              <span />
              <span>Total</span>
              <span className={s.tipValue}>{format(geom.totals[hover]!)}</span>
            </div>
          </div>
        )}
      </div>
      {n > 0 && (
        <details className={s.tableToggle}>
          <summary>Show as table</summary>
          <table className={s.table} aria-label={`${label} table`}>
            <thead>
              <tr>
                <th scope="col">Date</th>
                {series.map((ser) => (
                  <th key={ser.name} scope="col">
                    {ser.name}
                  </th>
                ))}
                <th scope="col">Total</th>
              </tr>
            </thead>
            <tbody>
              {tableRows.map((i) => (
                <tr key={dates[i]}>
                  <td>{formatDate(dates[i]!)}</td>
                  {series.map((ser) => (
                    <td key={ser.name} className="num">
                      {ser.values[i] === null || ser.values[i] === undefined ? "—" : format(ser.values[i]!)}
                    </td>
                  ))}
                  <td className="num">{format(geom.totals[i]!)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
    </figure>
  );
}
