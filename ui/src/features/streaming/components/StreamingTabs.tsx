import { NavLink } from "react-router-dom";
import { RadioTower, Gauge } from "lucide-react";
import { cn } from "@/lib/utils";

const streamingTabs = [
  {
    to: "/streaming/topics",
    label: "Topics",
    icon: RadioTower,
  },
  {
    to: "/streaming/offsets",
    label: "Consumers",
    icon: Gauge,
  },
];

export function StreamingTabs() {
  return (
    <div className="flex items-center gap-2 overflow-x-auto rounded-lg border bg-card p-1">
      {streamingTabs.map((tab) => (
        <NavLink
          key={tab.to}
          to={tab.to}
          className={({ isActive }) =>
            cn(
              "inline-flex min-w-fit items-center gap-1.5 rounded-md px-3 py-1.5 text-sm transition-colors",
              isActive
                ? "bg-primary/10 text-primary"
                : "text-muted-foreground hover:bg-muted hover:text-foreground",
            )
          }
        >
          <tab.icon className="h-4 w-4" />
          <span>{tab.label}</span>
        </NavLink>
      ))}
    </div>
  );
}

