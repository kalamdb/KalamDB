import { useNavigate, useLocation } from "react-router-dom";
import { useAuth } from "@/lib/auth";
import { useEffect } from "react";
import LoginForm from "@/components/auth/LoginForm";
import AuthSplitLayout from "@/components/auth/AuthSplitLayout";

export default function Login() {
  const { isAuthenticated } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();

  const from = (location.state as { from?: { pathname: string } })?.from?.pathname || "/dashboard";

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
      description="Sign in with an account that has DBA or system privileges."
      panelTitle="Realtime Data for AI Agents"
      panelDescription="Store agent memory, chat history, and tool calls. Stream live updates. Isolate per-tenant data with USER tables. Run SQL in SQL Studio and explore per-user namespaces."
      panelFootnote="Enterprise Admin UI"
    >
      <LoginForm onSuccess={handleLoginSuccess} returnTo={from} />
    </AuthSplitLayout>
  );
}
