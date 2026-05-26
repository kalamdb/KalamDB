import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { AlertCircle, ExternalLink, KeyRound, User } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { authApi, type AuthLoginOptions, type OidcLoginOptions } from "@/lib/api";
import { buildOAuthAuthorizationUrl } from "@/lib/oauth";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface LoginFormProps {
  onSuccess?: () => void;
  returnTo?: string;
}

export default function LoginForm({ onSuccess, returnTo = "/dashboard" }: LoginFormProps) {
  const { login, error, isLoading } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const [isRedirecting, setIsRedirecting] = useState(false);
  const [loginOptions, setLoginOptions] = useState<AuthLoginOptions>({
    local: { enabled: true },
    oidc: null,
  });

  useEffect(() => {
    let cancelled = false;
    authApi
      .loginOptions()
      .then((options) => {
        if (!cancelled) {
          setLoginOptions(options);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setLoginOptions({ local: { enabled: true }, oidc: null });
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const oidcOption = useMemo<OidcLoginOptions | null>(() => {
    const oidc = loginOptions.oidc;
    if (!oidc?.enabled || !oidc.authorization_endpoint || !oidc.token_endpoint) {
      return null;
    }
    return oidc;
  }, [loginOptions.oidc]);

  const oidcConfigurationError = useMemo(() => {
    const oidc = loginOptions.oidc;
    if (!oidc?.enabled || oidcOption) {
      return null;
    }

    return `${oidc.display_name} login is enabled on the server, but the Admin UI could not load the provider login endpoints. Check that ${oidc.issuer}/.well-known/openid-configuration is reachable from the KalamDB server.`;
  }, [loginOptions.oidc, oidcOption]);

  const localLoginEnabled = loginOptions.local.enabled;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLocalError(null);

    if (!username || !password) {
      setLocalError("Please enter username and password");
      return;
    }

    try {
      await login({ user: username, password });
      onSuccess?.();
    } catch {
      // Error is already handled by auth context
    }
  };

  const displayError = localError || error;

  const handleOAuthLogin = async (provider: OidcLoginOptions) => {
    setLocalError(null);
    setIsRedirecting(true);
    try {
      window.location.assign(await buildOAuthAuthorizationUrl(provider, returnTo));
    } catch (err) {
      setLocalError(err instanceof Error ? err.message : "External login failed");
      setIsRedirecting(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="flex flex-col gap-4">
      {oidcOption && (
        <>
          <Button
            type="button"
            variant="secondary"
            className="h-11 w-full"
            disabled={isLoading || isRedirecting}
            onClick={() => void handleOAuthLogin(oidcOption)}
          >
            <ExternalLink data-icon="inline-start" />
            Continue with {oidcOption.display_name}
          </Button>

          {localLoginEnabled && <div className="flex items-center gap-3 py-2">
            <div className="h-px flex-1 bg-border" />
            <span className="text-xs font-medium text-muted-foreground">OR</span>
            <div className="h-px flex-1 bg-border" />
          </div>}
        </>
      )}

      {displayError && (
        <Alert variant="destructive">
          <AlertCircle className="h-4 w-4" />
          <AlertDescription>{displayError}</AlertDescription>
        </Alert>
      )}

      {oidcConfigurationError && !displayError && (
        <Alert>
          <AlertCircle className="h-4 w-4" />
          <AlertDescription>{oidcConfigurationError}</AlertDescription>
        </Alert>
      )}

      {localLoginEnabled && <>
      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-2">
          <label htmlFor="username" className="text-sm font-medium">
            Username
          </label>
          <div className="relative">
            <User className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              id="username"
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="Enter username"
              disabled={isLoading}
              autoComplete="username"
              autoFocus
              className="h-11 pl-10"
            />
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <label htmlFor="password" className="text-sm font-medium">
            Password
          </label>
          <div className="relative">
            <KeyRound className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              id="password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="Enter password"
              disabled={isLoading}
              autoComplete="current-password"
              className="h-11 pl-10"
            />
          </div>
        </div>
      </div>

      <Button type="submit" className="h-11 w-full" disabled={isLoading}>
        {isLoading ? "Signing in..." : "Log in"}
      </Button>
      </>}

      <div className="flex flex-col gap-1 text-center text-sm text-muted-foreground">
        <p>
          Need setup on an unconfigured node?{" "}
          <Link to="/setup" className="font-medium text-primary hover:underline">
            Run setup
          </Link>
        </p>
        <p className="text-xs">
          Nodes started with scripts/cluster.sh are already configured. Sign in as root with the configured root password.
        </p>
      </div>
    </form>
  );
}
