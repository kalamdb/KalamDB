import { useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { AlertCircle } from "lucide-react";
import AuthSplitLayout from "@/components/auth/AuthSplitLayout";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { useAuth } from "@/lib/auth";
import { consumeOAuthRedirect } from "@/lib/oauth";

export default function OAuthCallback() {
  const navigate = useNavigate();
  const { loginWithExternalToken } = useAuth();
  const handled = useRef(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (handled.current) {
      return;
    }
    handled.current = true;

    const completeLogin = async () => {
      try {
        const result = await consumeOAuthRedirect(window.location.hash, window.location.search);
        await loginWithExternalToken(result.token);
        navigate(result.returnTo, { replace: true });
      } catch (err) {
        setError(err instanceof Error ? err.message : "External login failed");
      }
    };

    void completeLogin();
  }, [loginWithExternalToken, navigate]);

  return (
    <AuthSplitLayout
      description="Completing external sign in."
      panelTitle="Realtime Data for AI Agents"
      panelDescription="Store agent memory, chat history, and tool calls. Stream live updates. Isolate per-tenant data with USER tables. Run SQL in SQL Studio and explore per-user namespaces."
      panelFootnote="Enterprise Admin UI"
    >
      <div className="space-y-5">
        {error ? (
          <>
            <Alert variant="destructive">
              <AlertCircle />
              <AlertDescription>{error}</AlertDescription>
            </Alert>
            <Button asChild className="h-11 w-full">
              <Link to="/login">Back to login</Link>
            </Button>
          </>
        ) : (
          <div className="flex items-center justify-center gap-3 py-8 text-muted-foreground">
            <Spinner className="size-5" />
            <span>Signing in...</span>
          </div>
        )}
      </div>
    </AuthSplitLayout>
  );
}