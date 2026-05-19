import React from "react";
import { UserCircle2, Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

// The demo's sign-in dropdown.
//
// AppShell picks a default demo user on first render (see DEFAULT_USER
// there), so this component never auto-selects — it just fetches the list
// of demo users at GET /api/users to populate the dropdown options. The
// user can switch identities anytime: open two browser tabs, pick alice in
// one and bob in the other, and confirm the per-user partitioning keeps
// their data isolated.
//
// THIS IS A DEMO PICKER, NOT REAL AUTH. Passwords live server-side; anyone
// who picks "alice" becomes alice. The server's production fence (see
// server/index.ts, assertProductionFence) refuses to boot under
// NODE_ENV=production unless ALLOW_UNAUTHENTICATED_TOKENS=true is set, so
// this can't deploy as real auth by accident.

interface UserPickerProps {
  currentUser: string;
  onUserChange: (next: string) => void;
}

export function UserPicker({ currentUser, onUserChange }: UserPickerProps) {
  const [users, setUsers] = React.useState<string[]>([]);
  const [open, setOpen] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const rootRef = React.useRef<HTMLDivElement | null>(null);

  React.useEffect(() => {
    // `alive` guards every setState below — `signal.aborted` alone catches
    // mid-fetch unmount, but a slow response that arrives AFTER unmount with
    // a 2xx status would still call setUsers. StrictMode mounts effects
    // twice in dev, so the guard also avoids a "set state on unmounted
    // component" warning on the first throwaway mount.
    let alive = true;
    const ctrl = new AbortController();
    fetch("/api/users", { signal: ctrl.signal })
      .then(async (res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const body = (await res.json()) as { users?: unknown };
        const list = Array.isArray(body.users)
          ? body.users.filter((u): u is string => typeof u === "string")
          : [];
        if (alive) setUsers(list);
      })
      .catch((err) => {
        if (!alive || ctrl.signal.aborted) return;
        setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      alive = false;
      ctrl.abort();
    };
  }, []);

  // Click-outside closes the menu. Keyboard Esc handled by the buttons'
  // default behavior + onBlur via tabindex management isn't reliable across
  // browsers, so we keep this simple.
  React.useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      const root = rootRef.current;
      if (!root) return;
      if (!root.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const pick = (next: string): void => {
    setOpen(false);
    if (next === currentUser) return;
    onUserChange(next);
  };

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className={cn(
          "w-full flex items-center gap-2 px-3 py-2 rounded-lg border text-sm",
          "border-[var(--surface-border)] bg-[var(--surface)]/40",
          "hover:bg-[var(--surface)] transition",
        )}
      >
        <UserCircle2 className="size-4 opacity-70 shrink-0" />
        <span className="flex-1 text-left truncate">
          Signed in as <span className="font-medium">{currentUser}</span>
        </span>
        <ChevronDown className={cn("size-3.5 opacity-60 transition", open && "rotate-180")} />
      </button>
      {open && (
        <ul
          role="listbox"
          className={cn(
            "absolute bottom-full left-0 right-0 mb-1 z-10",
            "rounded-lg border border-[var(--surface-border)] bg-[var(--background-alt)]",
            "shadow-lg overflow-hidden",
          )}
        >
          {users.map((u) => (
            <Option key={u} label={u} selected={currentUser === u} onClick={() => pick(u)} />
          ))}
          {users.length === 0 && (
            <li className="px-3 py-2 text-xs text-[var(--muted-foreground)]">
              {error ? `Could not load users (${error})` : "Loading users…"}
            </li>
          )}
        </ul>
      )}
    </div>
  );
}

interface OptionProps {
  label: string;
  selected: boolean;
  onClick: () => void;
}

function Option({ label, selected, onClick }: OptionProps) {
  return (
    <li>
      <button
        type="button"
        role="option"
        aria-selected={selected}
        onClick={onClick}
        className={cn(
          "w-full flex items-center gap-2 px-3 py-2 text-sm text-left",
          "hover:bg-[var(--surface)] transition",
          selected && "bg-[var(--surface)]",
        )}
      >
        <Check className={cn("size-3.5", selected ? "opacity-100" : "opacity-0")} />
        <span className="truncate">{label}</span>
      </button>
    </li>
  );
}
