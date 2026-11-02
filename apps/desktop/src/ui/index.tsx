import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Link } from "react-router";
import s from "./ui.module.css";

function cx(...names: (string | false | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}

export function Card(props: { title?: ReactNode; aside?: ReactNode; label?: string; id?: string; children: ReactNode }) {
  return (
    <section className={s.card} aria-label={props.label} id={props.id}>
      {(props.title || props.aside) && (
        <div className={s.cardHeader}>
          {props.title && <h2 className={s.cardTitle}>{props.title}</h2>}
          {props.aside}
        </div>
      )}
      {props.children}
    </section>
  );
}

export type Tone = "neutral" | "accent" | "warn" | "info";

export function Badge(props: { tone?: Tone; children: ReactNode }) {
  return <span className={cx(s.badge, s[`tone-${props.tone ?? "neutral"}`])}>{props.children}</span>;
}

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  count?: number;
}

export function Segmented<T extends string>(props: {
  label: string;
  options: SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div role="group" aria-label={props.label} className={s.segmented}>
      {props.options.map((o) => (
        <button
          key={o.value}
          type="button"
          className={s.segment}
          aria-pressed={o.value === props.value}
          onClick={() => props.onChange(o.value)}
        >
          {o.label}
          {o.count !== undefined && <span className={s.segmentCount}>{o.count}</span>}
        </button>
      ))}
    </div>
  );
}

/** A semantic table that scrolls horizontally inside its own box. */
export function DataTable(props: { label: string; minWidth?: number; children: ReactNode }) {
  return (
    <div className={s.tableScroll}>
      <table className={s.table} aria-label={props.label} style={{ minWidth: props.minWidth }}>
        {props.children}
      </table>
    </div>
  );
}

export function EmptyState(props: { title: string; message: ReactNode; action?: { to: string; label: string } }) {
  return (
    <div className={s.empty} role="status">
      <h3 className={s.emptyTitle}>{props.title}</h3>
      <p className={s.emptyMessage}>{props.message}</p>
      {props.action && (
        <Link className={cx(s.button, s["button-secondary"])} to={props.action.to}>
          {props.action.label}
        </Link>
      )}
    </div>
  );
}

export function Button({
  variant = "secondary",
  small,
  className,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "primary" | "secondary" | "ghost"; small?: boolean }) {
  return (
    <button
      type="button"
      className={cx(s.button, s[`button-${variant}`], small && s["button-small"], className)}
      {...rest}
    />
  );
}
export { LineChart, type ChartSeries, type LineChartProps } from "./LineChart";
export { StackedAreaChart, type StackedAreaChartProps, type StackedSeries } from "./StackedAreaChart";
export { Donut, type DonutSlice } from "./Donut";
export { CATEGORICAL, slotColor } from "./palette";
