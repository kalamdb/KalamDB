import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { RefreshCw, Search } from "lucide-react";
import { PageLayout } from "@/components/layout/PageLayout";
import { StreamingTabs } from "@/features/streaming/components/StreamingTabs";
import { useGetStreamingConsumerGroupsQuery } from "@/store/apiSlice";
import { buildGroupSqlSnippet } from "@/features/streaming/sql";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatTimestamp } from "@/lib/formatters";

function formatNullableTimestamp(value: string | null): string {
  if (!value) {
    return "-";
  }
  return formatTimestamp(value, undefined, "iso8601-datetime", "utc");
}

export default function StreamingGroups() {
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const {
    data: groups = [],
    isFetching,
    error,
    refetch,
  } = useGetStreamingConsumerGroupsQuery();

  const filteredGroups = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) {
      return groups;
    }
    return groups.filter((group) => group.groupId.toLowerCase().includes(query));
  }, [groups, search]);

  const errorMessage =
    error && "error" in error && typeof error.error === "string"
      ? error.error
      : error
        ? "Failed to load consumer groups"
        : null;

  return (
    <PageLayout
      title="Streaming"
      description="Consumer group diagnostics"
      actions={(
        <Button variant="outline" size="sm" onClick={() => void refetch()} disabled={isFetching}>
          <RefreshCw className={`mr-1.5 h-4 w-4 ${isFetching ? "animate-spin" : ""}`} />
          Refresh
        </Button>
      )}
    >
      <StreamingTabs />

      <div className="relative max-w-sm">
        <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
        <Input
          placeholder="Search groups..."
          className="pl-9"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
      </div>

      {errorMessage && (
        <Card className="border-destructive/30 bg-destructive/5">
          <CardContent className="pt-4 text-sm text-destructive">{errorMessage}</CardContent>
        </Card>
      )}

      <Card>
        <CardHeader className="pb-2">
          <CardTitle>Consumer Groups</CardTitle>
          <CardDescription>
            {filteredGroups.length} group{filteredGroups.length === 1 ? "" : "s"} visible
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Group</TableHead>
                  <TableHead>Topics</TableHead>
                  <TableHead>Partitions</TableHead>
                  <TableHead>Last Activity</TableHead>
                  <TableHead className="w-[220px]">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredGroups.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={5} className="py-8 text-center text-muted-foreground">
                      {search.trim() ? "No groups match your search." : "No consumer groups found."}
                    </TableCell>
                  </TableRow>
                ) : (
                  filteredGroups.map((group) => (
                    <TableRow key={group.groupId}>
                      <TableCell className="font-mono text-xs">{group.groupId}</TableCell>
                      <TableCell>{group.topicCount}</TableCell>
                      <TableCell>{group.partitionCount}</TableCell>
                      <TableCell className="font-mono text-xs">{formatNullableTimestamp(group.lastUpdatedAt)}</TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() =>
                              navigate("/sql", {
                                state: {
                                  prefillSql: buildGroupSqlSnippet(group.groupId),
                                  prefillTitle: `Group ${group.groupId}`,
                                },
                              })
                            }
                          >
                            Open SQL
                          </Button>
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() =>
                              navigate(`/streaming/offsets?group=${encodeURIComponent(group.groupId)}`)
                            }
                          >
                            View Offsets
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  ))
                )}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>
    </PageLayout>
  );
}

