import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { AlertCircle, Eye, EyeOff, ExternalLink, KeyRound, User } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { authApi, type AuthLoginOptions, type OidcLoginOptions } from "@/lib/api";
import { buildOAuthAuthorizationUrl } from "@/lib/oauth";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";

interface LoginFormProps {
  onSuccess?: () => void;
  returnTo?: string;
}

export default function LoginForm({ onSuccess, returnTo = "/dashboard" }: LoginFormProps) {
  const { login, error, isLoading } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
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
    <form onSubmit={handleSubmit} className="flex flex-col gap-5">
      {oidcOption && (
        <>
          <Button
            type="button"
            variant="outline"
            className="h-9 w-full text-sm"
            disabled={isLoading || isRedirecting}
            onClick={() => void handleOAuthLogin(oidcOption)}
          >
            {isRedirecting ? <Spinner data-icon="inline-start" /> : <ExternalLink data-icon="inline-start" />}
            Continue with {oidcOption.display_name}
          </Button>

          {localLoginEnabled && (
            <div className="flex items-center gap-3">
              <div className="h-px flex-1 bg-border" />
              <span className="text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground">or</span>
              <div className="h-px flex-1 bg-border" />
            </div>
          )}
        </>
      )}

      {displayError && (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertDescription>{displayError}</AlertDescription>
        </Alert>
      )}

      {localLoginEnabled && (
        <>
          <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-2">
              <Label htmlFor="username">Username</Label>
              <div className="relative">
                <User className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  id="username"
                  type="text"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  placeholder="root"
                  disabled={isLoading}
                  autoComplete="username"
                  autoFocus
                  className="h-9 pl-9 text-sm md:text-sm"
                />
              </div>
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="password">Password</Label>
              <div className="relative">
                <KeyRound className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  id="password"
                  type={showPassword ? "text" : "password"}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="Enter password"
                  disabled={isLoading}
                  autoComplete="current-password"
                  className="h-9 pl-9 pr-10 text-sm md:text-sm"
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={showPassword ? "Hide password" : "Show password"}
                  aria-pressed={showPassword}
                  disabled={isLoading}
                  onClick={() => setShowPassword((visible) => !visible)}
                  className="absolute right-1.5 top-1/2 -translate-y-1/2 text-muted-foreground"
                >
                  {showPassword ? <EyeOff /> : <Eye />}
                </Button>
              </div>
            </div>
          </div>

          <Button type="submit" className="h-9 w-full text-sm" disabled={isLoading}>
            {isLoading && <Spinner data-icon="inline-start" />}
            {isLoading ? "Signing in..." : "Log in"}
          </Button>
        </>
      )}

      {oidcConfigurationError && !displayError && (
        <Alert className="border-warning/40 bg-warning/5">
          <AlertCircle className="text-warning" />
          <AlertDescription>{oidcConfigurationError}</AlertDescription>
        </Alert>
      )}

      <div className="space-y-1 border-t pt-4">
        <p className="text-sm text-muted-foreground">
          Unconfigured node?{" "}
          <Link to="/setup" className="font-medium text-primary hover:underline">
            Run setup
          </Link>
        </p>
        <p className="text-xs leading-relaxed text-muted-foreground">
          Nodes started with scripts/cluster.sh are already configured. Sign in as root with the configured root
          password.
        </p>
      </div>
    </form>
  );
}
