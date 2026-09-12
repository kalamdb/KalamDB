import { Link, Navigate, useNavigate, useParams } from "react-router-dom";
import { ArrowLeft, RefreshCw } from "lucide-react";
import { FunctionLogs } from "@/components/functions/FunctionLogs";
import { FunctionOverview } from "@/components/functions/FunctionOverview";
import { FunctionRevisions } from "@/components/functions/FunctionRevisions";
import { FunctionTest } from "@/components/functions/FunctionTest";
import { PageLayout } from "@/components/layout/PageLayout";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { rtkErrorMessage } from "@/features/functions/display";
import { functionDetailPath, functionListPath } from "@/features/functions/paths";
import { listItemsFromCatalog } from "@/services/functionsService";
import { FUNCTION_DETAIL_TABS, isFunctionDetailTab, type FunctionDetailTab } from "@/features/functions/types";
import { useGetProcedureCatalogQuery } from "@/store/apiSlice";
import { useMemo } from "react";

const TAB_LABELS: Record<FunctionDetailTab, string> = {
  overview: "Overview",
  logs: "Logs",
  revisions: "Revisions",
  test: "Test",
};

export default function FunctionDetail() {
  const navigate = useNavigate();
  const params = useParams<{ procedureId: string; tab?: string }>();
  const procedureId = decodeURIComponent(params.procedureId ?? "");
  const tab = params.tab && isFunctionDetailTab(params.tab) ? params.tab : "overview";

  const { data: catalog, isFetching, error, refetch } = useGetProcedureCatalogQuery();
  const items = useMemo(() => (catalog ? listItemsFromCatalog(catalog) : []), [catalog]);
  const procedure = items.find((item) => item.id.toLowerCase() === procedureId.toLowerCase()) ?? null;
  const errorMessage = rtkErrorMessage(error, "Failed to load function catalog");

  if (params.tab && !isFunctionDetailTab(params.tab) && procedureId) {
    return <Navigate to={functionDetailPath(procedureId)} replace />;
  }

  return (
    <PageLayout
      title={procedure?.id ?? procedureId}
      description={procedure?.comment || "Inspect this server function."}
      actions={(
        <Button variant="outline" size="sm" onClick={() => void refetch()} disabled={isFetching}>
          <RefreshCw className={`h-4 w-4 ${isFetching ? "animate-spin" : ""}`} />
          Refresh
        </Button>
      )}
    >
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Link to={functionListPath()} className="inline-flex items-center gap-1 hover:text-foreground">
          <ArrowLeft className="size-3.5" />
          Functions
        </Link>
        <span>/</span>
        <span className="text-foreground">{procedure?.id ?? procedureId}</span>
      </div>

      {errorMessage ? <p className="text-sm text-destructive">{errorMessage}</p> : null}

      {!catalog && isFetching ? (
        <p className="text-sm text-muted-foreground">Loading function…</p>
      ) : !catalog || !procedure ? (
        <p className="text-sm text-muted-foreground">Procedure {procedureId} was not found in the catalog.</p>
      ) : (
        <Tabs
          value={tab}
          onValueChange={(next) => {
            if (isFunctionDetailTab(next)) {
              navigate(functionDetailPath(procedure.id, next));
            }
          }}
        >
          <TabsList variant="line">
            {FUNCTION_DETAIL_TABS.map((item) => (
              <TabsTrigger key={item} value={item}>
                {TAB_LABELS[item]}
              </TabsTrigger>
            ))}
          </TabsList>
          <TabsContent value="overview" className="mt-4">
            <FunctionOverview procedure={procedure} snapshot={catalog} />
          </TabsContent>
          <TabsContent value="logs" className="mt-4">
            <FunctionLogs procedureId={procedure.id} />
          </TabsContent>
          <TabsContent value="revisions" className="mt-4">
            <FunctionRevisions moduleId={procedure.moduleId} revisions={catalog.revisions} />
          </TabsContent>
          <TabsContent value="test" className="mt-4">
            {tab === "test" ? <FunctionTest procedure={procedure} /> : null}
          </TabsContent>
        </Tabs>
      )}
    </PageLayout>
  );
}
