import { Navigate, useNavigate, useParams } from "react-router-dom";
import { RefreshCw } from "lucide-react";
import { FunctionLogs } from "@/components/functions/FunctionLogs";
import { FunctionOverview } from "@/components/functions/FunctionOverview";
import { FunctionRevisions } from "@/components/functions/FunctionRevisions";
import { FunctionTest } from "@/components/functions/FunctionTest";
import { PageBreadcrumb } from "@/components/layout/PageBreadcrumb";
import { PageLayout } from "@/components/layout/PageLayout";
import { PageTabs, PageTabsList } from "@/components/layout/PageTabs";
import { Button } from "@/components/ui/button";
import { TabsContent, TabsTrigger } from "@/components/ui/tabs";
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

  const { data: catalog, isFetching, error, refetch } = useGetProcedureCatalogQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const items = useMemo(() => (catalog ? listItemsFromCatalog(catalog) : []), [catalog]);
  const procedure = items.find((item) => item.id.toLowerCase() === procedureId.toLowerCase()) ?? null;
  const errorMessage = rtkErrorMessage(error, "Failed to load function catalog");
  const title = procedure?.id ?? procedureId;

  if (params.tab && !isFunctionDetailTab(params.tab) && procedureId) {
    return <Navigate to={functionDetailPath(procedureId)} replace />;
  }

  return (
    <PageLayout
      breadcrumb={(
        <PageBreadcrumb
          items={[
            { label: "Functions", to: functionListPath() },
            { label: title, mono: true },
          ]}
        />
      )}
      title={<span className="font-mono">{title}</span>}
      description={procedure?.comment || "Inspect this server function."}
      actions={(
        <Button variant="outline" size="sm" onClick={() => void refetch()} disabled={isFetching}>
          <RefreshCw data-icon="inline-start" className={isFetching ? "animate-spin" : undefined} />
          Refresh
        </Button>
      )}
    >
      {errorMessage ? <p className="text-sm text-destructive">{errorMessage}</p> : null}

      {!catalog && isFetching ? (
        <p className="text-sm text-muted-foreground">Loading function…</p>
      ) : !catalog || !procedure ? (
        <p className="text-sm text-muted-foreground">Procedure {procedureId} was not found in the catalog.</p>
      ) : (
        <PageTabs
          value={tab}
          onValueChange={(next) => {
            if (isFunctionDetailTab(next)) {
              navigate(functionDetailPath(procedure.id, next));
            }
          }}
        >
          <PageTabsList>
            {FUNCTION_DETAIL_TABS.map((item) => (
              <TabsTrigger key={item} value={item}>
                {TAB_LABELS[item]}
              </TabsTrigger>
            ))}
          </PageTabsList>
          <TabsContent value="overview">
            <FunctionOverview procedure={procedure} snapshot={catalog} />
          </TabsContent>
          <TabsContent value="logs">
            <FunctionLogs procedureId={procedure.id} moduleId={procedure.moduleId} />
          </TabsContent>
          <TabsContent value="revisions">
            <FunctionRevisions moduleId={procedure.moduleId} revisions={catalog.revisions} />
          </TabsContent>
          <TabsContent value="test">
            {tab === "test" ? <FunctionTest procedure={procedure} /> : null}
          </TabsContent>
        </PageTabs>
      )}
    </PageLayout>
  );
}
