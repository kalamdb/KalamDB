export type ToolPlan = {
  toolName: string;
  requiresApproval: boolean;
  approvalTitle: string;
  approvalBody: string;
};

export function createToolPlan(message: string): ToolPlan {
  const lower = message.toLowerCase();
  const requiresApproval = lower.includes('migrate')
    || lower.includes('database')
    || lower.includes('customer')
    || lower.includes('refund')
    || lower.includes('approval');

  if (requiresApproval) {
    return {
      toolName: 'human_approval',
      requiresApproval: true,
      approvalTitle: 'Action Required',
      approvalBody: lower.includes('project alpha')
        ? 'Run database migration for Project Alpha?'
        : 'Approve the assistant before it performs this customer-facing or data-changing step.',
    };
  }

  return {
    toolName: lower.includes('deploy') || lower.includes('release')
      ? 'release_lookup'
      : lower.includes('analysis') || lower.includes('outlier') ? 'analysis_sandbox' : 'conversation_search',
    requiresApproval: false,
    approvalTitle: '',
    approvalBody: '',
  };
}

export function buildApprovalMessage(message: string): string {
  const subject = message.trim() || 'this request';
  return `I can continue with "${subject}", but this step needs a human decision to approve or decline before the agent proceeds.`;
}

export function buildAssistantReply(message: string): string {
  const subject = message.trim() || 'the latest request';
  const lower = subject.toLowerCase();

  if (lower.includes('approved') || lower.includes('approval granted')) {
    return 'Approval received. I continued the database migration plan, verified the dependent checks, and recorded the next safe action for Project Alpha.';
  }

  if (lower.includes('outlier') || lower.includes('analysis') || lower.includes('spreadsheet')) {
    return 'I checked the spreadsheet, isolated the European market outliers, and prepared a compact summary for the Project Alpha workspace.';
  }

  if (lower.includes('deploy') || lower.includes('release')) {
    return `I reviewed "${subject}" with the release context and found the next deployment check to run.`;
  }

  return `I reviewed "${subject}" against the conversation context and prepared the next concise step.`;
}

export function splitIntoTokenChunks(text: string, chunkSize = 24): string[] {
  const chunks: string[] = [];
  for (let index = 0; index < text.length; index += chunkSize) {
    chunks.push(text.slice(index, index + chunkSize));
  }
  return chunks;
}