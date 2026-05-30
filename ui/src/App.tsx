import { Suspense, lazy } from "react";
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { Loader2 } from "lucide-react";
import { AuthProvider } from "./lib/auth";
import { BackendStatusProvider } from "./lib/backend-status";
import { SqlPreviewProvider } from "./components/sql-preview";
import { Toaster } from "./components/ui/toaster-provider";
import ProtectedRoute from "./components/auth/ProtectedRoute";
import SetupGuard from "./components/auth/SetupGuard";
import Layout from "./components/layout/Layout";

const Login = lazy(() => import("./pages/Login"));
const OAuthCallback = lazy(() => import("./pages/OAuthCallback"));
const SetupWizard = lazy(() => import("./pages/SetupWizard"));
const Dashboard = lazy(() => import("./pages/Dashboard"));
const SqlStudio = lazy(() => import("./pages/SqlStudio"));
const Users = lazy(() => import("./pages/Users"));
const Jobs = lazy(() => import("./pages/Jobs"));
const LiveQueries = lazy(() => import("./pages/LiveQueries"));
const Logging = lazy(() => import("./pages/Logging"));
const Settings = lazy(() => import("./pages/Settings"));
const StreamingTopics = lazy(() => import("./pages/StreamingTopics"));
const StreamingTopicDetail = lazy(() => import("./pages/StreamingTopicDetail"));
const StreamingOffsets = lazy(() => import("./pages/StreamingOffsets"));

function RouteFallback() {
  return (
    <div className="flex h-full min-h-0 items-center justify-center bg-background text-muted-foreground">
      <Loader2 className="h-5 w-5 animate-spin" />
    </div>
  );
}

function App() {
  return (
    <BrowserRouter basename="/ui">
      <AuthProvider>
        <BackendStatusProvider>
          <SqlPreviewProvider>
           <Toaster>
            <div className="flex h-[100dvh] min-h-0 flex-col overflow-hidden bg-background">
              <div className="min-h-0 flex-1 overflow-hidden">
                <SetupGuard>
                  <Suspense fallback={<RouteFallback />}>
                    <Routes>
                      <Route path="/setup" element={<SetupWizard />} />
                      <Route path="/login" element={<Login />} />
                      <Route path="/oauth/callback" element={<OAuthCallback />} />
                      <Route
                        path="/"
                        element={
                          <ProtectedRoute>
                            <Layout />
                          </ProtectedRoute>
                        }
                      >
                        <Route index element={<Navigate to="/dashboard" replace />} />
                        <Route path="dashboard" element={<Dashboard />} />
                        <Route path="sql" element={<SqlStudio />} />
                        <Route path="users" element={<Users />} />
                        <Route path="jobs" element={<Jobs />} />
                        <Route path="live-queries" element={<LiveQueries />} />
                        <Route path="streaming" element={<Navigate to="/streaming/topics" replace />} />
                        <Route path="streaming/topics" element={<StreamingTopics />} />
                        <Route path="streaming/topics/:topicId" element={<StreamingTopicDetail />} />
                        <Route path="streaming/groups" element={<Navigate to="/streaming/offsets" replace />} />
                        <Route path="streaming/offsets" element={<StreamingOffsets />} />
                        <Route path="logging" element={<Logging />} />
                        <Route path="logging/:tab" element={<Logging />} />
                        <Route path="settings" element={<Settings />} />
                        <Route path="settings/:category" element={<Settings />} />
                      </Route>
                    </Routes>
                  </Suspense>
                </SetupGuard>
              </div>
            </div>
           </Toaster>
          </SqlPreviewProvider>
        </BackendStatusProvider>
      </AuthProvider>
    </BrowserRouter>
  );
}

export default App;
