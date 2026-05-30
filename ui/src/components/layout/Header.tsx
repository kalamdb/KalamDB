import { Database, Search } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import BackendStatusIndicator from "./BackendStatusIndicator";
import { NotificationsDropdown } from "./NotificationsDropdown";
import { UserMenu } from "./UserMenu";

const iconUrl = `${import.meta.env.BASE_URL}favicon.png`;

export default function Header() {
  const { user, logout } = useAuth();

  return (
    <header className="sticky top-0 z-40 flex h-12 shrink-0 items-center gap-3 border-b bg-background px-3 text-foreground">
      <div className="flex shrink-0 items-center gap-2">
        <img
          src={iconUrl}
          alt="KalamDB"
          className="h-6 w-6 shrink-0 object-contain"
        />
        <Button variant="ghost" size="xs" className="h-[26px] px-2 text-xs font-normal">
          KalamDB Admin
        </Button>
        <Button variant="outline" size="xs" className="hidden h-[26px] gap-1.5 px-2 text-xs font-normal md:inline-flex">
          <Database data-icon="inline-start" />
          Connect
        </Button>
      </div>

      <div className="ml-auto hidden min-w-0 items-center md:flex">
        <div className="relative w-40 lg:w-56">
          <Search className="pointer-events-none absolute left-2 top-2 h-3.5 w-3.5 text-muted-foreground" />
          <Input
            placeholder="Search..."
            className="h-[30px] border-input bg-background pl-7 pr-10 text-xs"
          />
          <kbd className="pointer-events-none absolute right-2 top-1.5 hidden rounded-sm border px-1 text-[10px] text-muted-foreground lg:block">
            Cmd K
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
