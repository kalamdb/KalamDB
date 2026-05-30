import { useParams, useNavigate } from "react-router-dom";
import { AuditLogList } from "@/components/audit/AuditLogList";
import { JobList } from "@/components/jobs/JobList";
import { ServerLogList } from "@/components/logs/ServerLogList";
import { SlowQueriesLogList } from "@/components/logs/SlowQueriesLogList";
import { PageHeader } from "@/components/layout/typography";
import { cn } from "@/lib/utils";

type LogTab = "audit" | "jobs" | "server" | "slow-queries";

const tabs: { id: LogTab; label: string; description: string }[] = [
  { id: "audit", label: "Audit", description: "View system audit trail and activity history" },
  { id: "server", label: "Server", description: "View real-time server logs and debug information" },
  { id: "jobs", label: "Jobs", description: "View and monitor background jobs in the system" },
  { id: "slow-queries", label: "Slow Queries", description: "Inspect queries captured in system.slow_queries" },
];

export default function Logging() {
  const { tab } = useParams<{ tab?: string }>();
  const navigate = useNavigate();
  
  // Default to server logs if no tab specified
  const activeTab = (tab as LogTab) || "server";

  const handleTabChange = (tabId: LogTab) => {
    navigate(`/logging/${tabId}`);
  };

  const renderContent = () => {
    switch (activeTab) {
      case "audit":
        return <AuditLogList />;
      case "server":
        return <ServerLogList />;
      case "jobs":
        return <JobList />;
      case "slow-queries":
        return <SlowQueriesLogList />;
      default:
        return <ServerLogList />;
    }
  };

  const activeTabData = tabs.find((t) => t.id === activeTab) || tabs[0];

  return (
    <div className="flex flex-col h-full">
      <div className="border-b px-4 lg:px-6 pt-4 pb-4">
        <PageHeader title="Logs & Analytics" description={activeTabData.description} />
      </div>

      <div className="flex flex-1 min-h-0 overflow-hidden">
        <aside className="w-52 shrink-0 border-r p-3">
          <nav className="space-y-1" aria-label="Logs and analytics sections">
            {tabs.map((tabItem) => (
              <button
                key={tabItem.id}
                onClick={() => handleTabChange(tabItem.id)}
                className={cn(
                  "w-full rounded-md px-3 py-2 text-left text-sm font-medium transition-colors",
                  activeTab === tabItem.id
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent/60 hover:text-foreground"
                )}
              >
                {tabItem.label}
              </button>
            ))}
          </nav>
        </aside>

        <div className="flex-1 min-h-0 p-4 lg:p-6">
          {renderContent()}
        </div>
      </div>
    </div>
  );
}
