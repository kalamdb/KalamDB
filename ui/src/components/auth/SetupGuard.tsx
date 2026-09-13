import { useEffect, type ReactNode } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { Database } from "lucide-react";
import { Spinner } from "@/components/ui/spinner";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import { checkSetupStatus } from "@/store/setupSlice";

interface SetupGuardProps {
  children: ReactNode;
}

export default function SetupGuard({ children }: SetupGuardProps) {
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const location = useLocation();
  const { needsSetup, isCheckingStatus } = useAppSelector((state) => state.setup);

  // Check setup status on mount
  useEffect(() => {
    dispatch(checkSetupStatus());
  }, [dispatch]);

  // Redirect based on setup status
  useEffect(() => {
    if (isCheckingStatus) return;

    const isOnSetupPage = location.pathname === "/setup";

    if (needsSetup === true && !isOnSetupPage) {
      // Server needs setup, redirect to setup page
      navigate("/setup", { replace: true });
    } else if (needsSetup === false && isOnSetupPage) {
      // Server is already set up, redirect to login
      navigate("/login", { replace: true });
    }
  }, [needsSetup, isCheckingStatus, location.pathname, navigate]);

  // Show loading while checking status
  if (isCheckingStatus) {
    return (
      <div className="flex h-full min-h-full flex-col items-center justify-center bg-background">
        <div className="flex items-center gap-3 mb-4">
          <Database className="h-10 w-10 text-primary" />
          <span className="text-2xl font-bold">KalamDB</span>
        </div>
        <div className="flex items-center gap-2 text-muted-foreground">
          <Spinner className="size-5" />
          <span>Checking server status...</span>
        </div>
      </div>
    );
  }

  return <>{children}</>;
}
