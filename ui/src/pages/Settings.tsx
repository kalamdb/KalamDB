import { NavLink, useParams } from "react-router-dom";
import {
  Archive,
  HardDrive,
  Network,
  Settings as SettingsIcon,
  Shield,
  Sliders,
  UserCircle2,
} from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { SettingsView } from "@/components/settings/SettingsView";
import { useAuth } from "@/lib/auth";
import { cn } from "@/lib/utils";
import Cluster from "./Cluster";
import Storages from "./Storages";

type SettingsSection = "all" | "general" | "cluster" | "storages" | "backup" | "security";

interface SettingsLink {
  id: SettingsSection;
  name: string;
  href: string;
  icon: typeof SettingsIcon;
}

const settingsLinks: SettingsLink[] = [
  { id: "all", name: "All Settings", href: "/settings", icon: SettingsIcon },
  { id: "general", name: "General", href: "/settings/general", icon: Sliders },
  { id: "cluster", name: "Cluster", href: "/settings/cluster", icon: Network },
  { id: "storages", name: "Storages", href: "/settings/storages", icon: HardDrive },
  { id: "backup", name: "Backup", href: "/settings/backup", icon: Archive },
  { id: "security", name: "Security", href: "/settings/security", icon: Shield },
];

const validSections = new Set<SettingsSection>(settingsLinks.map((link) => link.id));

function resolveActiveSection(category?: string): SettingsSection {
  return category && validSections.has(category as SettingsSection)
    ? (category as SettingsSection)
    : "all";
}

function SettingsSidebar() {
  return (
    <aside className="hidden h-full w-64 shrink-0 flex-col border-r bg-background md:flex">
      <div className="flex h-12 shrink-0 items-center border-b px-6">
        <p className="text-base font-semibold text-foreground">Settings</p>
      </div>
      <nav aria-label="Settings sections" className="min-h-0 flex-1 overflow-y-auto px-3 py-4">
        <p className="mb-2 px-3 text-[11px] font-medium uppercase text-muted-foreground">Configuration</p>
        <div className="flex flex-col gap-0.5">
          {settingsLinks.map((link) => (
            <NavLink
              key={link.id}
              to={link.href}
              end={link.id === "all"}
              className={({ isActive }) => cn(
                "flex h-7 items-center gap-2 rounded-md px-3 text-sm font-medium transition-colors",
                isActive
                  ? "bg-accent text-foreground"
                  : "text-muted-foreground hover:bg-accent/80 hover:text-accent-foreground",
              )}
            >
              <link.icon className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{link.name}</span>
            </NavLink>
          ))}
        </div>
      </nav>
    </aside>
  );
}

function MobileSettingsNav() {
  return (
    <nav aria-label="Settings mobile links" className="mb-6 overflow-x-auto md:hidden">
      <div className="flex min-w-max gap-1 border-b pb-2">
        {settingsLinks.map((link) => (
          <NavLink
            key={link.id}
            to={link.href}
            end={link.id === "all"}
            className={({ isActive }) => cn(
              "inline-flex h-8 items-center gap-2 rounded-md px-3 text-sm font-medium transition-colors",
              isActive
                ? "bg-accent text-foreground"
                : "text-muted-foreground hover:bg-accent/80 hover:text-accent-foreground",
            )}
          >
            <link.icon className="h-3.5 w-3.5 shrink-0" />
            {link.name}
          </NavLink>
        ))}
      </div>
    </nav>
  );
}

function CurrentUserCard() {
  const { user } = useAuth();

  return (
    <Card className="max-w-3xl">
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <UserCircle2 className="h-4 w-4" />
          Current User
        </CardTitle>
        <CardDescription>Your current admin session information</CardDescription>
      </CardHeader>
      <CardContent className="pt-4">
        <dl className="grid gap-x-6 gap-y-3 text-sm sm:grid-cols-[140px_minmax(0,1fr)]">
          <dt className="text-muted-foreground">Username</dt>
          <dd className="min-w-0 truncate text-foreground">{user?.username ?? "-"}</dd>
          <dt className="text-muted-foreground">Role</dt>
          <dd className="min-w-0 truncate text-foreground">{user?.role ?? "-"}</dd>
          <dt className="text-muted-foreground">Email</dt>
          <dd className="min-w-0 truncate text-foreground">{user?.email || "-"}</dd>
          <dt className="text-muted-foreground">User ID</dt>
          <dd className="min-w-0 truncate font-mono text-xs text-foreground">{user?.id ?? "-"}</dd>
        </dl>
      </CardContent>
    </Card>
  );
}

function EmptySettingsCard({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <Card className="max-w-3xl">
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent>
        <p className="text-sm text-muted-foreground">No settings available yet.</p>
      </CardContent>
    </Card>
  );
}

export default function Settings() {
  const { category } = useParams<{ category?: string }>();
  const activeSection = resolveActiveSection(category);

  const renderContent = () => {
    switch (activeSection) {
      case "all":
        return (
          <div className="flex max-w-5xl flex-col gap-6">
            <CurrentUserCard />
            <SettingsView />
          </div>
        );
      case "general":
        return (
          <div className="flex max-w-5xl flex-col gap-6">
            <CurrentUserCard />
            <EmptySettingsCard
              title="General Settings"
              description="General configuration options for this KalamDB admin UI."
            />
          </div>
        );
      case "cluster":
        return <Cluster />;
      case "storages":
        return <Storages />;
      case "backup":
        return (
          <Card className="max-w-5xl">
            <CardHeader className="border-b">
              <CardTitle>Backup Settings</CardTitle>
              <CardDescription>Configure backup and restore options.</CardDescription>
            </CardHeader>
            <CardContent className="pt-4">
              <SettingsView filterCategory="backup" />
            </CardContent>
          </Card>
        );
      case "security":
        return (
          <EmptySettingsCard
            title="Security Settings"
            description="Configure authentication and authorization."
          />
        );
    }
  };

  return (
    <div className="flex h-full min-h-0 overflow-hidden bg-background">
      <SettingsSidebar />
      <main className="min-w-0 flex-1 overflow-auto bg-background">
        <div className="px-4 py-6 sm:px-6 md:px-10 md:py-10">
          <MobileSettingsNav />
          <div className="mb-6 flex items-start justify-between gap-4">
            <div className="min-w-0">
              <h1 className="text-2xl font-normal leading-8 text-foreground">Settings</h1>
              <p className="text-sm text-muted-foreground">
                View and configure KalamDB admin settings.
              </p>
            </div>
          </div>
          {renderContent()}
        </div>
      </main>
    </div>
  );
}
