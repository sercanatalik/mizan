import type { ReactNode } from "react";
import { NavLink, Outlet } from "react-router";
import { usePendingDecisions } from "../bridge/queries";
import { SECTIONS, type Section } from "./sections";
import { useMarketEvents } from "./useMarketEvents";
import { useMenuEvents } from "./useMenuEvents";
import s from "./layout.module.css";

function Icon({ d }: { d: string }) {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <path d={d} />
    </svg>
  );
}

function NavItem({ section, badge }: { section: Section; badge?: number }) {
  return (
    <NavLink to={section.path} end={section.path === "/"} className={s.navLink}>
      <Icon d={section.icon} />
      <span className={s.navLabel}>{section.label}</span>
      {badge ? (
        <span className={s.navBadge} aria-label={`${badge} awaiting decision`}>
          {badge}
        </span>
      ) : null}
    </NavLink>
  );
}

export function Sidebar() {
  const pending = usePendingDecisions().data ?? 0;
  const groups = ["Portfolio", "Data"] as const;
  return (
    <nav aria-label="Main" className={s.sidebar}>
      <NavLink to="/" className={s.brand} aria-label="Mizan home">
        <svg aria-hidden="true" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--accent-on-dark)" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
          <path d="M12 3v17M8 20h8M4 7h16M4 7l-2.5 6h5zM20 7l-2.5 6h5z" />
        </svg>
        mizan
      </NavLink>
      {groups.map((g) => (
        <div key={g} className={s.group}>
          <div className={s.groupLabel}>{g}</div>
          {SECTIONS.filter((x) => x.group === g).map((x) => (
            <NavItem key={x.id} section={x} badge={x.id === "imports" ? pending : undefined} />
          ))}
        </div>
      ))}
      <div className={s.bottom}>
        {SECTIONS.filter((x) => x.group === null).map((x) => (
          <NavItem key={x.id} section={x} />
        ))}
      </div>
    </nav>
  );
}

export function Layout() {
  useMenuEvents();
  useMarketEvents();
  return (
    <div className={s.shell}>
      <Sidebar />
      <main className={s.content}>
        <div className={s.contentInner}>
          <Outlet />
        </div>
      </main>
    </div>
  );
}

export function PageHeader(props: { title: string; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <header className={s.pageHeader}>
      <div>
        <h1 className={s.pageTitle}>{props.title}</h1>
        {props.subtitle && <div className={s.pageSubtitle}>{props.subtitle}</div>}
      </div>
      {props.actions}
    </header>
  );
}
