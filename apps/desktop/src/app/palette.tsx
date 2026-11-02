import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useNavigate } from "react-router";
import { SECTIONS } from "./sections";
import s from "./palette.module.css";

export interface PaletteItem {
  id: string;
  label: string;
  hint?: string;
  /** Extra text that matches but is not shown (ISIN, identifiers). */
  search?: string;
  shortcut?: string;
  run: () => void;
}

export interface PaletteGroup {
  name: string;
  items: PaletteItem[];
  /** Shown before anything is typed. */
  always?: boolean;
}

interface PaletteCtx {
  open: boolean;
  show: () => void;
  hide: () => void;
}

const Ctx = createContext<PaletteCtx>({ open: false, show: () => {}, hide: () => {} });

export const usePalette = () => useContext(Ctx);

/** Fold case and accents so "is portfoy" finds "İş Portföy". */
export function fold(text: string): string {
  return text
    .replace(/İ/g, "i")
    .replace(/I/g, "i")
    .replace(/ı/g, "i")
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase();
}

/** Filter and rank: prefix matches on the label first, then other matches; groups by their best match. */
export function matchGroups(groups: PaletteGroup[], query: string): PaletteGroup[] {
  const q = fold(query.trim());
  if (!q) return groups.filter((g) => g.always && g.items.length > 0);
  return groups
    .map((g) => {
      const scored = g.items
        .map((it, i) => {
          const label = fold(it.label);
          const all = `${label} ${fold(it.hint ?? "")} ${fold(it.search ?? "")}`;
          const rank = label.startsWith(q) ? 0 : label.split(/\s+/).some((w) => w.startsWith(q)) ? 1 : all.includes(q) ? 2 : -1;
          return { it, i, rank };
        })
        .filter((x) => x.rank >= 0)
        .sort((a, b) => a.rank - b.rank || a.i - b.i);
      return { group: { ...g, items: scored.map((x) => x.it) }, best: scored[0]?.rank ?? 99 };
    })
    .filter((g) => g.group.items.length > 0)
    // Groups with the best match come first, so the top result is the closest one.
    .map((g, i) => ({ ...g, i }))
    .sort((a, b) => a.best - b.best || a.i - b.i)
    .map((g) => g.group);
}

export function PaletteProvider({ children, groups }: { children: ReactNode; groups: (hide: () => void) => PaletteGroup[] }) {
  const [open, setOpen] = useState(false);
  const show = useCallback(() => setOpen(true), []);
  const hide = useCallback(() => setOpen(false), []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const value = useMemo(() => ({ open, show, hide }), [open, show, hide]);
  return (
    <Ctx.Provider value={value}>
      {children}
      {open && <CommandPalette groups={groups(hide)} onClose={hide} />}
    </Ctx.Provider>
  );
}

/** The Go to group: every sidebar section. */
export function useGoToGroup(): PaletteGroup {
  const navigate = useNavigate();
  return {
    name: "Go to",
    always: true,
    items: SECTIONS.map((x) => ({
      id: `go:${x.id}`,
      label: x.label,
      shortcut: x.shortcut ? `⌘${x.shortcut}` : undefined,
      run: () => navigate(x.path),
    })),
  };
}

function CommandPalette({ groups, onClose }: { groups: PaletteGroup[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const returnTo = useRef<Element | null>(null);
  const shown = matchGroups(groups, query);
  const flat = shown.flatMap((g) => g.items);

  useEffect(() => {
    returnTo.current = document.activeElement;
    input.current?.focus();
    return () => {
      const el = returnTo.current;
      if (el instanceof HTMLElement) el.focus();
    };
  }, []);

  const runAt = (i: number) => {
    const it = flat[i];
    if (!it) return;
    onClose();
    it.run();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSel((i) => Math.min(i + 1, flat.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSel((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      runAt(sel);
    }
  };

  const offsets = shown.map((_, gi) => shown.slice(0, gi).reduce((n, g) => n + g.items.length, 0));
  return (
    <div className={s.backdrop} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label="Search or run a command" className={s.dialog} onKeyDown={onKeyDown}>
        <div className={s.searchRow}>
          <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <circle cx="11" cy="11" r="7" />
            <path d="M20 20l-3.5-3.5" />
          </svg>
          <input
            ref={input}
            className={s.input}
            aria-label="Search"
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-results"
            aria-activedescendant={flat[sel] ? `palette-${flat[sel].id}` : undefined}
            placeholder="Search screens, accounts, assets, actions…"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSel(0);
            }}
          />
          <kbd className={s.kbd}>esc</kbd>
        </div>
        <div id="palette-results" role="listbox" aria-label="Results" className={s.results}>
          {shown.map((g, gi) => (
            <div key={g.name} role="group" aria-label={g.name}>
              <div className={s.groupName}>{g.name}</div>
              {g.items.map((it, ii) => {
                const i = (offsets[gi] ?? 0) + ii;
                return (
                  <div
                    key={it.id}
                    id={`palette-${it.id}`}
                    role="option"
                    aria-selected={i === sel}
                    className={s.option}
                    onMouseEnter={() => setSel(i)}
                    onClick={() => runAt(i)}
                  >
                    <span className={s.label}>
                      {it.label}
                      {it.hint && <span className={s.hint}> {it.hint}</span>}
                    </span>
                    {it.shortcut && <span className={s.shortcut}>{it.shortcut}</span>}
                  </div>
                );
              })}
            </div>
          ))}
          {flat.length === 0 && <div className={s.none}>No match. Try an account name, a symbol or “import”.</div>}
        </div>
        <div className={s.footer}>
          <span>↑↓ move</span>
          <span>↵ open</span>
          <span>esc close</span>
        </div>
      </div>
    </div>
  );
}
