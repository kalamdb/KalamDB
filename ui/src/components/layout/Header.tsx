import { useEffect, useRef } from "react";
import { Search } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { Input } from "@/components/ui/input";
import BackendStatusIndicator from "./BackendStatusIndicator";
import { NotificationsDropdown } from "./NotificationsDropdown";
import { UserMenu } from "./UserMenu";

const isAppleDevice =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad|iPod/.test(navigator.userAgent);

export default function Header() {
  const { user, logout } = useAuth();
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    function focusSearch(event: KeyboardEvent): void {
      if (event.key.toLowerCase() !== "k" || !(event.metaKey || event.ctrlKey)) {
        return;
      }

      event.preventDefault();
      searchRef.current?.focus();
      searchRef.current?.select();
    }

    window.addEventListener("keydown", focusSearch);
    return () => window.removeEventListener("keydown", focusSearch);
  }, []);

  return (
    <header className="sticky top-0 z-40 flex h-14 shrink-0 items-center gap-3 bg-background px-4 text-foreground">
      <div className="mx-auto hidden min-w-0 items-center md:flex">
        <div className="relative w-72 lg:w-[420px]">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            ref={searchRef}
            type="search"
            aria-label="Search tables, queries and logs"
            placeholder="Search tables, queries, logs..."
            className="h-9 rounded-full pl-9 pr-16"
          />
          <kbd className="pointer-events-none absolute right-3 top-1/2 hidden -translate-y-1/2 rounded-md border px-1.5 py-0.5 text-[10px] text-muted-foreground lg:block">
            {isAppleDevice ? "⌘" : "Ctrl"} K
          </kbd>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2">
        <BackendStatusIndicator />
        <NotificationsDropdown />
        <UserMenu
          username={user?.username ?? "User"}
          role={user?.role ?? "user"}
          onLogout={logout}
        />
      </div>
    </header>
  );
}
