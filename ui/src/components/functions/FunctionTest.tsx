import { useEffect, useMemo, useState } from "react";
import { Play } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { CodeBlock } from "@/components/ui/code-block";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { FunctionParameterInput } from "@/components/functions/FunctionParameterInput";
import { displayLogLevel, formatDurationMs } from "@/features/functions/format";
import { formatLogTime, logLevelClassName, rtkErrorMessage } from "@/features/functions/display";
import {
  buildInvocationBody,
  defaultTestValues,
  setPathValue,
  validateTestValues,
  type TestFieldError,
  type TestValue,
} from "@/features/functions/testValues";
import type { ProcedureLogRecord, ProcedureMetadata } from "@/features/functions/types";
import { logsForExecution } from "@/services/functionsService";
import {
  useGetProcedureLogsQuery,
  useInvokeProcedureMutation,
} from "@/store/apiSlice";

export function FunctionTest({ procedure }: { procedure: ProcedureMetadata }) {
  const { user } = useAuth();
  const [values, setValues] = useState<Record<string, TestValue>>(() => defaultTestValues(procedure));
  const [errors, setErrors] = useState<TestFieldError[]>([]);
  const [startedAt, setStartedAt] = useState<string | null>(null);
  const [invokeProcedure, invokeState] = useInvokeProcedureMutation();
  const { data: logs = [], refetch } = useGetProcedureLogsQuery({
    procedureId: procedure.id,
    limit: 200,
  });

  useEffect(() => {
    setValues(defaultTestValues(procedure));
    setErrors([]);
    setStartedAt(null);
  }, [procedure.id]);

  const result = invokeState.data;
  const invokeError = rtkErrorMessage(invokeState.error, "Failed to invoke procedure");
  const executionLogs = useMemo(() => {
    if (!startedAt) {
      return [] as ProcedureLogRecord[];
    }
    return logsForExecution(logs, procedure.id, startedAt);
  }, [logs, procedure.id, startedAt]);

  const runTest = async () => {
    const nextErrors = validateTestValues(procedure, values);
    setErrors(nextErrors);
    if (nextErrors.length > 0) {
      return;
    }

    const body = buildInvocationBody(procedure, values);
    try {
      const response = await invokeProcedure({
        schema: procedure.schema,
        name: procedure.name,
        body,
      }).unwrap();
      setStartedAt(response.startedAt);
      void refetch();
    } catch {
      setStartedAt(new Date().toISOString());
      void refetch();
    }
  };

  const durationMs =
    executionLogs.find((log) => log.outcome === "ok" || log.outcome === "error")?.durationMs ||
    result?.durationMs ||
    null;

  return (
    <div className="grid gap-4 xl:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Input</CardTitle>
          <CardDescription>Run this procedure with catalog-typed arguments.</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4">
          <div className="grid gap-1.5">
            <span className="text-xs font-medium">Run as user</span>
            <div className="rounded-md border px-3 py-2 text-sm">
              {user?.username || user?.id || "current session"}
            </div>
            <p className="text-xs text-muted-foreground">
              HTTP invocation uses the signed-in Admin session. Impersonation is not available on this path.
            </p>
          </div>

          {procedure.parameters.length === 0 ? (
            <p className="text-sm text-muted-foreground">This procedure has no parameters.</p>
          ) : (
            procedure.parameters.map((parameter) => (
              <FunctionParameterInput
                key={parameter.name}
                name={parameter.name}
                path={parameter.name}
                type={parameter.type}
                value={values[parameter.name]}
                errors={errors}
                onChange={(path, next) => {
                  setValues((current) => setPathValue(current, path, next));
                  setErrors((current) => current.filter((error) => error.path !== path && !error.path.startsWith(`${path}.`) && !error.path.startsWith(`${path}[`)));
                }}
              />
            ))
          )}

          <TooltipProvider delayDuration={200}>
            <div className="flex items-start justify-between gap-3 rounded-md border px-3 py-2">
              <div className="grid gap-0.5">
                <span className="text-xs font-medium">Roll back database changes after test</span>
                <span className="text-xs text-muted-foreground">
                  HTTP invocation commits in its own transaction, so this cannot be enabled yet.
                </span>
              </div>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span>
                    <Switch disabled checked={false} aria-label="Roll back database changes after test" />
                  </span>
                </TooltipTrigger>
                <TooltipContent className="max-w-xs">
                  Test rollback is unavailable because POST /v1/functions always commits its own transaction.
                </TooltipContent>
              </Tooltip>
            </div>
          </TooltipProvider>

          {errors.length > 0 ? (
            <p className="text-xs text-destructive">{errors.length} field{errors.length === 1 ? "" : "s"} need attention.</p>
          ) : null}

          <Button type="button" onClick={() => void runTest()} disabled={invokeState.isLoading}>
            <Play className="size-3.5" />
            {invokeState.isLoading ? "Running…" : "Run test"}
          </Button>
        </CardContent>
      </Card>

      <div className="grid gap-4">
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Response</CardTitle>
            <CardDescription>Function execution result.</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid grid-cols-3 gap-2">
              <Metric label="Status" value={result ? (result.ok ? String(result.statusCode) : result.errorCode || String(result.statusCode)) : "—"} />
              <Metric label="Duration" value={formatDurationMs(durationMs)} />
              <Metric label="Memory" value="—" />
            </div>
            <div className="grid gap-1.5">
              <span className="text-xs font-medium">Response output</span>
              {result ? (
                <CodeBlock value={result.ok ? result.body : result.body ?? result.errorMessage} jsonPreferred />
              ) : (
                <pre className="rounded-md border bg-muted/30 px-3 py-2 font-mono text-xs text-muted-foreground">
                  Run the test to see the response…
                </pre>
              )}
            </div>
            {invokeError ? <p className="text-sm text-destructive">{invokeError}</p> : null}
            {result && !result.ok && result.errorMessage ? (
              <p className="text-sm text-destructive">{result.errorMessage}</p>
            ) : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Execution logs</CardTitle>
            <CardDescription>Logs associated with this test run, if available.</CardDescription>
          </CardHeader>
          <CardContent>
            {executionLogs.length === 0 ? (
              <pre className="rounded-md border bg-muted/30 px-3 py-2 font-mono text-xs text-muted-foreground">
                Run the test to see logs…
              </pre>
            ) : (
              <ol className="grid gap-1 font-mono text-xs">
                {executionLogs.map((log) => (
                  <li key={`${log.timestamp}-${log.executionId}-${log.message}`}>
                    <span className="text-muted-foreground">{formatLogTime(log.timestamp)}</span>{" "}
                    <span className={logLevelClassName(log.level)}>{displayLogLevel(log.level)}</span>{" "}
                    <span>{log.message || "—"}</span>
                  </li>
                ))}
              </ol>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border px-3 py-2">
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className="text-sm">{value}</div>
    </div>
  );
}
