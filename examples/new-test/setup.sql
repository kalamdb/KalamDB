CREATE NAMESPACE IF NOT EXISTS blog;

CREATE SHARED TABLE IF NOT EXISTS blog.blogs (
    blog_id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    content TEXT NOT NULL,
    summary TEXT,
    created TIMESTAMP NOT NULL DEFAULT NOW(),
    updated TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE SHARED TABLE IF NOT EXISTS blog.summary_failures (
    run_key TEXT PRIMARY KEY,
    blog_id TEXT NOT NULL,
    error TEXT NOT NULL,
    created TIMESTAMP NOT NULL DEFAULT NOW(),
    updated TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TOPIC blog.summarizer;
ALTER TOPIC blog.summarizer ADD SOURCE blog.blogs ON INSERT WITH (payload = 'full');
ALTER TOPIC blog.summarizer ADD SOURCE blog.blogs ON UPDATE WITH (payload = 'full');

INSERT INTO blog.blogs (content, summary)
VALUES (
    'KalamDB can route table mutations into topics in real time. This enables lightweight agents that consume events and write enriched data back immediately.',
    NULL
);
