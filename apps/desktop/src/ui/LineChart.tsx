import { useId, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import s from "./chart.module.css";

/** Validated two-slot palette (teal, violet) on the light surface. */
const SERIES_COLORS = ["#139a82", "#6250d6"] as const;

export interface ChartSeries {
  name: string;
  /** One value per date; `null` leaves a gap. */
  values: (number | null)[];
  /** Text shown in the tooltip and table, one per date (defaults to the formatted value). */
  labels?: (string | null)[];
}

export interface LineChartProps {
  /** Accessible name of the chart. */
  label: string;
  dates: string[];
  /** One or two series; the first is the primary one. */
  series: ChartSeries[];
  /** Days that use estimated values; shaded behind the lines. */
  estimated?: boolean[];
  format: (v: number) => string;
  /** Format a date for the axis, tooltip and table. */
  formatDate?: (d: string) => string;
  height?: number;
}

const W = 720;
const PAD = { top: 12, right: 12, bottom: 26, left: 64 };

function niceStep(range: number, count: number): number {
  const raw = range / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const n = raw / mag;
  return (n <= 1 ? 1 : n <= 2 ? 2 : n <= 2.5 ? 2.5 : n <= 5 ? 5 : 10) * mag;
}

/** Runs of consecutive true flags as [start, end] index pairs. */
function runs(flags: boolean[]): [number, number][] {
  const out: [number, number][] = [];
  let start = -1;
  flags.forEach((f, i) => {
    if (f && start < 0) start = i;
    if (!f && start >= 0) {
      out.push([start, i - 1]);
      start = -1;
    }
  });
  if (start >= 0) out.push([start, flags.length - 1]);
  return out;
}

export function LineChart({ label, dates, series, estimated, format, formatDate = (d) => d, height = 220 }: LineChartProps) {
  const id = useId();
  const svgRef = useRef<SVGSVGElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const n = dates.length;

  const geom = useMemo(() => {
    const all = series.flatMap((x) => x.values).filter((v): v is number => v !== null);
    let lo = all.length ? Math.min(...all) : 0;
    let hi = all.length ? Math.max(...all) : 1;
    if (lo === hi) {
      lo -= Math.abs(lo) * 0.05 || 1;
      hi += Math.abs(hi) * 0.05 || 1;
    }
    const step = niceStep(hi - lo, 4);
    const y0 = Math.floor(lo / step) * step;
    const y1 = Math.ceil(hi / step) * step;
    const ticks: number[] = [];
    for (let v = y0; v <= y1 + step / 2; v += step) ticks.push(Number(v.toFixed(10)));
    const plotW = W - PAD.left - PAD.right;
    const plotH = height - PAD.top - PAD.bottom;
    const x = (i: number) => PAD.left + (n <= 1 ? plotW / 2 : (i / (n - 1)) * plotW);
    const y = (v: number) => PAD.top + plotH - ((v - y0) / (y1 - y0 || 1)) * plotH;
    const paths = series.map((ser) => {
      let d = "";
      let pen = false;
      ser.values.forEach((v, i) => {
        if (v === null) {
          pen = false;
          return;
        }
        d += `${pen ? "L" : "M"}${x(i).toFixed(1)},${y(v).toFixed(1)}`;
        pen = true;
      });
      return d;
    });
    return { ticks, x, y, paths, plotH };
  }, [series, n, height]);

  const shown = (si: number, i: number): string => {
    const ser = series[si]!;
    const l = ser.labels?.[i];
    if (l !== undefined) return l ?? "—";
    const v = ser.values[i];
    return v === null || v === undefined ? "—" : format(v);
  };

  const pick = (e: PointerEvent<SVGSVGElement>) => {
    const r = svgRef.current?.getBoundingClientRect();
    if (!r || n === 0) return;
    const px = ((e.clientX - r.left) / r.width) * W;
    const t = (px - PAD.left) / (W - PAD.left - PAD.right);
    setHover(Math.max(0, Math.min(n - 1, Math.round(t * (n - 1)))));
  };

  const onKey = (e: KeyboardEvent<SVGSVGElement>) => {
    if (n === 0) return;
    const cur = hover ?? n - 1;
    const step = e.shiftKey ? 30 : 1;
    if (e.key === "ArrowLeft") setHover(Math.max(0, cur - step));
    else if (e.key === "ArrowRight") setHover(Math.min(n - 1, cur + step));
    else if (e.key === "Home") setHover(0);
    else if (e.key === "End") setHover(n - 1);
    else if (e.key === "Escape") setHover(null);
    else return;
    e.preventDefault();
  };

  const lastIdx = (si: number) => {
    const v = series[si]!.values;
    for (let i = v.length - 1; i >= 0; i--) if (v[i] !== null) return i;
    return -1;
  };
  const summary =
    n === 0
      ? `${label}: no data`
      : `${label}, ${formatDate(dates[0]!)} to ${formatDate(dates[n - 1]!)}. ` +
        series.map((ser, si) => `${ser.name}: ${lastIdx(si) >= 0 ? shown(si, lastIdx(si)) : "no value"} at the end`).join("; ") +
        ".";

  const xLabels = n > 2 ? [0, Math.floor((n - 1) / 2), n - 1] : n === 2 ? [0, 1] : n === 1 ? [0] : [];
  const tableRows = useMemo(() => {
    // One row per month (its last day) plus the final day, so the table stays readable.
    const rows: number[] = [];
    dates.forEach((d, i) => {
      const next = dates[i + 1];
      if (!next || next.slice(0, 7) !== d.slice(0, 7)) rows.push(i);
    });
    return rows;
  }, [dates]);

  return (
    <figure className={s.figure} aria-labelledby={`${id}-sum`}>
      <figcaption id={`${id}-sum`} className={s.srOnly}>
        {summary}
      </figcaption>
      {series.length > 1 || estimated?.some(Boolean) ? (
        <ul className={s.legend} aria-label={`${label} legend`}>
          {series.map((ser, si) => (
            <li key={ser.name}>
              <span className={s.swatch} style={{ background: SERIES_COLORS[si] }} aria-hidden="true" />
              {ser.name}
            </li>
          ))}
          {estimated?.some(Boolean) && (
            <li>
              <span className={`${s.swatch} ${s.swatchBand}`} aria-hidden="true" />
              Estimated values
            </li>
          )}
        </ul>
      ) : null}
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
          {estimated &&
            runs(estimated).map(([a, b]) => (
              <rect
                key={a}
                data-testid="estimated-band"
                x={geom.x(a) - 1}
                y={PAD.top}
                width={Math.max(2, geom.x(b) - geom.x(a) + 2)}
                height={geom.plotH}
                className={s.band}
              />
            ))}
          {geom.ticks.map((t) => (
            <g key={t}>
              <line x1={PAD.left} x2={W - PAD.right} y1={geom.y(t)} y2={geom.y(t)} className={s.grid} />
              <text x={PAD.left - 8} y={geom.y(t)} className={s.tick} textAnchor="end" dominantBaseline="middle">
                {format(t)}
              </text>
            </g>
          ))}
          {xLabels.map((i, k) => (
            <text
              key={i}
              x={geom.x(i)}
              y={height - 6}
              className={s.tick}
              textAnchor={k === 0 ? "start" : k === xLabels.length - 1 ? "end" : "middle"}
            >
              {formatDate(dates[i]!)}
            </text>
          ))}
          {geom.paths.map((d, si) => (
            <path key={si} d={d} className={s.line} style={{ stroke: SERIES_COLORS[si] }} data-testid={`series-${si}`} />
          ))}
          {hover !== null && (
            <g aria-hidden="true">
              <line x1={geom.x(hover)} x2={geom.x(hover)} y1={PAD.top} y2={PAD.top + geom.plotH} className={s.crosshair} />
              {series.map((ser, si) => {
                const v = ser.values[hover];
                return v === null || v === undefined ? null : (
                  <circle key={si} cx={geom.x(hover)} cy={geom.y(v)} r={4.5} className={s.dot} style={{ fill: SERIES_COLORS[si] }} />
                );
              })}
            </g>
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
            <div className={s.tipDate}>
              {formatDate(dates[hover]!)}
              {estimated?.[hover] ? " · estimated" : ""}
            </div>
            {series.map((ser, si) => (
              <div key={ser.name} className={s.tipRow}>
                <span className={s.swatch} style={{ background: SERIES_COLORS[si] }} aria-hidden="true" />
                <span>{ser.name}</span>
                <span className={s.tipValue}>{shown(si, hover)}</span>
              </div>
            ))}
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
              </tr>
            </thead>
            <tbody>
              {tableRows.map((i) => (
                <tr key={dates[i]}>
                  <td>
                    {formatDate(dates[i]!)}
                    {estimated?.[i] ? " (estimated)" : ""}
                  </td>
                  {series.map((ser, si) => (
                    <td key={ser.name} className="num">
                      {shown(si, i)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
    </figure>
  );
}
