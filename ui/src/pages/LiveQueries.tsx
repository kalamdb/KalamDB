import { LiveQueryList } from '@/components/live-queries/LiveQueryList';
import { PageLayout } from "@/components/layout/PageLayout";

export default function LiveQueries() {
  return (
    <PageLayout
      title="Live Queries"
      description="Monitor active WebSocket subscriptions and live query connections"
    >
      <LiveQueryList />
    </PageLayout>
  );
}
