import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { AlertCircle, ArrowRight, Eye, EyeOff, ExternalLink, Lock, Terminal, User } from "lucide-react";
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
            className="h-11 w-full rounded-xl border-zinc-200 bg-white text-sm text-zinc-800 hover:bg-zinc-50 dark:border-zinc-200 dark:bg-white dark:text-zinc-800 dark:hover:bg-zinc-50"
            disabled={isLoading || isRedirecting}
            onClick={() => void handleOAuthLogin(oidcOption)}
          >
            {isRedirecting ? <Spinner data-icon="inline-start" /> : <ExternalLink data-icon="inline-start" />}
            Continue with {oidcOption.display_name}
          </Button>

          {localLoginEnabled && (
            <div className="flex items-center gap-3">
              <div className="h-px flex-1 bg-zinc-200" />
              <span className="text-[11px] font-medium uppercase tracking-[0.12em] text-zinc-500">or</span>
              <div className="h-px flex-1 bg-zinc-200" />
            </div>
          )}
        </>
      )}

      {displayError && (
        <Alert
          variant="destructive"
          className="border-red-200 bg-red-50 text-red-700 dark:border-red-200 dark:bg-red-50 dark:text-red-700 *:data-[slot=alert-description]:text-red-700"
        >
          <AlertCircle />
          <AlertDescription>{displayError}</AlertDescription>
        </Alert>
      )}

      {localLoginEnabled && (
        <>
          <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-2">
              <Label htmlFor="username" className="text-sm text-zinc-700">
                Username
              </Label>
              <div className="relative">
                <User className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-zinc-400" />
                <Input
                  id="username"
                  type="text"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  placeholder="root"
                  disabled={isLoading}
                  autoComplete="username"
                  autoFocus
                  className="h-11 rounded-xl border-zinc-200 bg-zinc-50/80 pl-10 text-sm focus-visible:border-emerald-500 focus-visible:ring-emerald-500/20 md:text-sm dark:border-zinc-200 dark:bg-zinc-50/80 dark:text-zinc-900 dark:placeholder:text-zinc-400"
                />
              </div>
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="password" className="text-sm text-zinc-700">
                Password
              </Label>
              <div className="relative">
                <Lock className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-zinc-400" />
                <Input
                  id="password"
                  type={showPassword ? "text" : "password"}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="Enter password"
                  disabled={isLoading}
                  autoComplete="current-password"
                  className="h-11 rounded-xl border-zinc-200 bg-zinc-50/80 pl-10 pr-11 text-sm focus-visible:border-emerald-500 focus-visible:ring-emerald-500/20 md:text-sm dark:border-zinc-200 dark:bg-zinc-50/80 dark:text-zinc-900 dark:placeholder:text-zinc-400"
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={showPassword ? "Hide password" : "Show password"}
                  aria-pressed={showPassword}
                  disabled={isLoading}
                  onClick={() => setShowPassword((visible) => !visible)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-400 hover:bg-zinc-100 hover:text-zinc-700 dark:text-zinc-400 dark:hover:bg-zinc-100 dark:hover:text-zinc-700"
                >
                  {showPassword ? <EyeOff /> : <Eye />}
                </Button>
              </div>
            </div>
          </div>

          <Button
            type="submit"
            className="group/submit h-11 w-full rounded-xl bg-gradient-to-r from-[#21a344] to-[#055441] text-sm font-semibold text-white shadow-[0_10px_24px_-12px_rgba(5,84,65,0.9)] hover:from-[#1d9440] hover:to-[#04483a]"
            disabled={isLoading}
          >
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
        <Alert className="border-amber-200 bg-amber-50/80 dark:border-amber-200 dark:bg-amber-50/80">
          <AlertCircle className="text-amber-600" />
          <AlertDescription className="text-amber-900/80">{oidcConfigurationError}</AlertDescription>
        </Alert>
      )}

      <div className="space-y-4 border-t border-zinc-100 pt-4">
        <p className="text-sm text-zinc-500">
          New here?{" "}
          <Link to="/setup" className="font-semibold text-[#077f4c] hover:underline">
            Run setup
          </Link>
        </p>

        <div className="flex gap-3 rounded-xl border border-zinc-200 bg-zinc-50/70 p-3">
          <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-[#055441] text-white">
            <Terminal className="size-4" />
          </span>
          <div className="space-y-1">
            <p className="text-sm font-semibold text-zinc-900">Unconfigured node?</p>
            <p className="text-xs leading-relaxed text-zinc-500">
              Nodes started with scripts/cluster.sh are already configured. Sign in as root with the configured root
              password.
            </p>
          </div>
        </div>
      </div>
    </form>
  );
}
