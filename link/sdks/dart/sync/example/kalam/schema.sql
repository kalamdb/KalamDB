CREATE NAMESPACE IF NOT EXISTS chat;

CREATE TABLE IF NOT EXISTS chat.conversations (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
) WITH (TYPE = 'USER');

CREATE TABLE IF NOT EXISTS chat.messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    role TEXT NOT NULL,
    author TEXT NOT NULL,
    text TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    delivered_at TIMESTAMP,
    read_at TIMESTAMP
) WITH (TYPE = 'USER');
