-- Migration: init
-- Created: 2026-06-24T00:00:00Z

-- UP
CREATE NAMESPACE IF NOT EXISTS react_ai_chat;

DROP TABLE IF EXISTS react_ai_chat.approval_actions;
DROP TABLE IF EXISTS react_ai_chat.typing_tokens;
DROP TABLE IF EXISTS react_ai_chat.approvals;
DROP TABLE IF EXISTS react_ai_chat.messages;
DROP TABLE IF EXISTS react_ai_chat.conversations;

CREATE USER TABLE IF NOT EXISTS react_ai_chat.conversations (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE USER TABLE IF NOT EXISTS react_ai_chat.messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    client_id TEXT,
    conversation_id TEXT NOT NULL,
    reply_to_message_id TEXT,
    role TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'sent',
    attachment FILE,
    approval_id TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE USER TABLE IF NOT EXISTS react_ai_chat.approvals (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE STREAM TABLE IF NOT EXISTS react_ai_chat.typing_tokens (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    status TEXT NOT NULL,
    token TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TTL_SECONDS = 120);

CREATE USER TABLE IF NOT EXISTS react_ai_chat.approval_actions (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    approval_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    action TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TOPIC IF NOT EXISTS react_ai_chat.agent_messages;
ALTER TOPIC react_ai_chat.agent_messages ADD SOURCE react_ai_chat.messages ON INSERT;

CREATE TOPIC IF NOT EXISTS react_ai_chat.agent_actions;
ALTER TOPIC react_ai_chat.agent_actions ADD SOURCE react_ai_chat.approval_actions ON INSERT;

INSERT INTO react_ai_chat.conversations (id, title, summary)
VALUES ('project-alpha', 'Project Alpha', 'Data analysis with approval-gated database migration');

INSERT INTO react_ai_chat.approvals (id, conversation_id, message_id, title, body, status)
VALUES ('approval-project-alpha', 'project-alpha', '1001', 'Action Required', 'Run database migration for Project Alpha?', 'pending');

INSERT INTO react_ai_chat.messages (id, client_id, conversation_id, role, body, status, approval_id)
VALUES (1001, NULL, 'project-alpha', 'assistant', 'I analyzed the historical datasets and generated the quarterly summary chart. The European market outliers are isolated and ready for review.', 'complete', 'approval-project-alpha');

-- DOWN
DROP TOPIC react_ai_chat.agent_actions;
DROP TOPIC react_ai_chat.agent_messages;
DROP TABLE IF EXISTS react_ai_chat.approval_actions;
DROP TABLE IF EXISTS react_ai_chat.typing_tokens;
DROP TABLE IF EXISTS react_ai_chat.approvals;
DROP TABLE IF EXISTS react_ai_chat.messages;
DROP TABLE IF EXISTS react_ai_chat.conversations;
