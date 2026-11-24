import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { AlertCircle, ArrowRight, Eye, EyeOff, ExternalLink, Lock, Terminal, User } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { authApi, type AuthLoginOptions, type OidcLoginOptions } from "@/lib/api";
import { buildOAuthAuthorizationUrl } from "@/lib/oauth";
import { cn } from "cn";
import { authButtonClass, authInputClass } from "@/components/auth/AuthSplitLayout";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
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
            className={cn(authButtonClass, "w-full")}
            disabled={isLoading || isRedirecting}
            onClick={() => void handleOAuthLogin(oidcOption)}
          >
            {isRedirecting ? <Spinner data-icon="inline-start" /> : <ExternalLink data-icon="inline-start" />}
            Continue with {oidcOption.display_name}
          </Button>

          {localLoginEnabled && (
            <div className="flex items-center gap-3">
              <Separator className="flex-1" />
              <span className="text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground">or</span>
              <Separator className="flex-1" />
            </div>
          )}
        </>
      )}

      {displayError && (
        <Alert variant="destructive" className="border-destructive/20 bg-destructive/5">
          <AlertCircle />
          <AlertDescription>{displayError}</AlertDescription>
        </Alert>
      )}

      {localLoginEnabled && (
        <>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="username" className="text-sm">
                Username
              </FieldLabel>
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
                  className={cn(authInputClass, "pl-10")}
                />
              </div>
            </Field>

            <Field>
              <FieldLabel htmlFor="password" className="text-sm">
                Password
              </FieldLabel>
              <div className="relative">
                <Lock className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  id="password"
                  type={showPassword ? "text" : "password"}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="Enter password"
                  disabled={isLoading}
                  autoComplete="current-password"
                  className={cn(authInputClass, "pl-10 pr-11")}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={showPassword ? "Hide password" : "Show password"}
                  aria-pressed={showPassword}
                  disabled={isLoading}
                  onClick={() => setShowPassword((visible) => !visible)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
                >
                  {showPassword ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
                </Button>
              </div>
            </Field>
          </FieldGroup>

          <Button type="submit" variant="brand" className={cn("group/submit w-full", authButtonClass)} disabled={isLoading}>
            {isLoading && <Spinner data-icon="inline-start" />}
            {isLoading ? "Signing in..." : "Log in"}
            {!isLoading && (
              <ArrowRight
                data-icon="inline-end"
                className="transition-transform group-hover/submit:translate-x-0.5"
              />
            )}
          </Button>
        </>
      )}

      {oidcConfigurationError && !displayError && (
        <Alert className="border-warning/40 bg-warning/10">
          <AlertCircle />
          <AlertDescription className="text-foreground/80">{oidcConfigurationError}</AlertDescription>
        </Alert>
      )}

      <div className="space-y-4 border-t pt-4">
        <p className="text-sm text-muted-foreground">
          New here?{" "}
          <Link to="/setup" className="font-semibold text-brand hover:underline">
            Run setup
          </Link>
        </p>

        <div className="flex gap-3 rounded-xl border bg-muted/50 p-3">
          <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-deep text-brand-foreground">
            <Terminal className="size-4" />
          </span>
          <div className="space-y-1">
            <p className="text-sm font-semibold text-foreground">Unconfigured node?</p>
            <p className="text-xs leading-relaxed text-muted-foreground">
              Nodes started with scripts/cluster.sh are already configured. Sign in as root with the configured root
              password.
            </p>
          </div>
        </div>
      </div>
    </form>
  );
}
