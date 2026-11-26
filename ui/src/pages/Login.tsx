import { useNavigate, useLocation } from "react-router-dom";
import { useAuth } from "@/lib/auth";
import { useEffect } from "react";
import { Code2, Database, Layers, Zap } from "lucide-react";
import LoginForm from "@/components/auth/LoginForm";
import AuthSplitLayout, { type AuthPanelFeature } from "@/components/auth/AuthSplitLayout";
import { safeReturnTo } from "@/lib/oauth";

const PANEL_FEATURES: AuthPanelFeature[] = [
  { label: "Live Updates", description: "Real-time data streams", icon: Zap },
  {
    label: "Per-Tenant Isolation",
    description: "USER tables & schemas",
    icon: Layers,
    iconClassName: "text-violet-500",
  },
  { label: "Agent Memory", description: "Persistent, queryable state", icon: Database, iconClassName: "text-teal-600" },
  { label: "SQL Studio", description: "Run and explore SQL", icon: Code2 },
];

export default function Login() {
  const { isAuthenticated } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();

  const from = safeReturnTo((location.state as { from?: { pathname: string } })?.from?.pathname);

  useEffect(() => {
    if (isAuthenticated) {
      navigate(from, { replace: true });
    }
  }, [isAuthenticated, navigate, from]);

  const handleLoginSuccess = () => {
    navigate(from, { replace: true });
  };

  return (
    <AuthSplitLayout
      heading="Realtime Data for AI Agents"
      headingHighlight="AI Agents"
      description="Store agent memory, chat history, and tool calls. Stream live updates and run SQL in a unified workspace."
      panelTitle="KalamDB for AI Agents"
      panelTitleHighlight="AI Agents"
      panelDescription="A realtime, SQL-native data platform to build stateful, multi-tenant AI systems."
      panelFootnote="Enterprise Admin UI"
      panelFeatures={PANEL_FEATURES}
    >
      <LoginForm onSuccess={handleLoginSuccess} returnTo={from} />
    </AuthSplitLayout>
  );
}
