import { Navigate, useLocation } from "react-router-dom";
import { useAuth } from "@/lib/auth";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

interface ProtectedRouteProps {
  children: React.ReactNode;
}

export default function ProtectedRoute({ children }: ProtectedRouteProps) {
  const { isAuthenticated, isLoading, user, logout } = useAuth();
  const location = useLocation();

  if (isLoading) {
    return (
      <div className="flex h-full min-h-full items-center justify-center text-muted-foreground">
        <Spinner className="size-5" />
      </div>
    );
  }

  if (!isAuthenticated) {
    return <Navigate to="/login" state={{ from: location }} replace />;
  }

  // Check role - only dba and system can access admin UI
  if (user && !["dba", "system"].includes(user.role)) {
    return (
      <div className="flex h-full min-h-full items-center justify-center">
        <div className="text-center">
          <h1 className="text-2xl font-bold text-destructive">Access Denied</h1>
          <p className="mt-2 text-muted-foreground">
            Admin UI access requires dba or system role.
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            Current role: <span className="font-mono">{user.role}</span>
          </p>
          <Button
            onClick={() => logout()}
            variant="outline"
            className="mt-4"
          >
            Logout and Switch User
          </Button>
        </div>
      </div>
    );
  }

  return <>{children}</>;
}
