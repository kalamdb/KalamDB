import { LiveQueryList } from '@/components/live-queries/LiveQueryList';
import { PageLayout } from "@/components/layout/PageLayout";
import { ReactLiveQueryDemo } from '@/components/live-data/ReactLiveQueryDemo';
import { AssistantWorkflowDemo } from '@/components/assistant/AssistantWorkflowDemo';

export default function LiveQueries() {
  return (
    <PageLayout
      title="Live Queries"
      description="Monitor active WebSocket subscriptions and live query connections"
    >
      <div className="mb-6 space-y-4">
        <ReactLiveQueryDemo />
        <AssistantWorkflowDemo />
      </div>
      <LiveQueryList />
    </PageLayout>
  );
}
