//! Shared FILE downloads follow SELECT row visibility.
//!
//! A caller receives the bytes only when a current row they can select stores that
//! FileRef. Hidden and missing paths use the same not-found body.

use kalam_client::models::ResponseStatus as LinkResponseStatus;
use kalamdb_api::http::sql::models::{ResponseStatus, SqlResponse};
use kalamdb_commons::Role;
use kalamdb_system::FileRef;
use reqwest::multipart;
use serde_json::{json, Value as JsonValue};
use serial_test::serial;
use uuid::Uuid;

use super::test_support::{
    auth_helper::create_user_auth_header_with_id, http_server::start_http_test_server,
};

fn unique_suffix() -> String {
    Uuid::new_v4().simple().to_string()
}

fn parse_file_ref(value: &JsonValue) -> anyhow::Result<FileRef> {
    if let Some(raw) = value.as_str() {
        return Ok(FileRef::try_from_json(raw)?);
    }
    Ok(serde_json::from_value(value.clone())?)
}

async fn execute_sql_multipart(
    server: &super::test_support::http_server::HttpTestServer,
    auth_header: &str,
    sql: &str,
    files: Vec<(&str, &str, &'static [u8], &str)>,
) -> anyhow::Result<SqlResponse> {
    let url = format!("{}/v1/api/sql", server.base_url());
    let client = reqwest::Client::new();
    let mut form = multipart::Form::new().text("sql", sql.to_string());
    for (field, filename, data, mime) in files {
        let part = multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str(mime)?;
        form = form.part(format!("file:{}", field), part);
    }
    let response = client
        .post(url)
        .header("Authorization", auth_header)
        .multipart(form)
        .send()
        .await?;
    Ok(response.json::<SqlResponse>().await?)
}

async fn must_sql(
    server: &super::test_support::http_server::HttpTestServer,
    sql: &str,
) -> anyhow::Result<()> {
    let resp = server.execute_sql(sql).await?;
    anyhow::ensure!(resp.status == LinkResponseStatus::Success, "{sql}: {:?}", resp.error);
    Ok(())
}

fn file_url(
    server: &super::test_support::http_server::HttpTestServer,
    namespace: &str,
    table: &str,
    file_ref: &FileRef,
) -> String {
    format!(
        "{}/v1/files/{}/{}/{}/{}",
        server.base_url(),
        namespace,
        table,
        file_ref.sub,
        file_ref.stored_name()
    )
}

async fn file_ref_at(
    server: &super::test_support::http_server::HttpTestServer,
    auth_header: &str,
    sql: &str,
    column: &str,
) -> anyhow::Result<FileRef> {
    let resp = server.execute_sql_with_auth(sql, auth_header).await?;
    anyhow::ensure!(resp.status == LinkResponseStatus::Success, "{sql}: {:?}", resp.error);
    let rows = resp.rows_as_maps();
    let value = rows
        .first()
        .and_then(|row| row.get(column))
        .ok_or_else(|| anyhow::anyhow!("missing {column} from {sql}"))?;
    parse_file_ref(value)
}

async fn get_with_auth(url: &str, auth_header: Option<&str>) -> anyhow::Result<reqwest::Response> {
    let client = reqwest::Client::new();
    let request = client.get(url);
    let request = if let Some(auth_header) = auth_header {
        request.header("Authorization", auth_header)
    } else {
        request
    };
    Ok(request.send().await?)
}

async fn assert_missing_file(url: &str, auth_header: &str) -> anyhow::Result<JsonValue> {
    let response = get_with_auth(url, Some(auth_header)).await?;
    anyhow::ensure!(
        response.status() == reqwest::StatusCode::NOT_FOUND,
        "expected 404 for {url}, got {}",
        response.status()
    );
    let body: JsonValue = response.json().await?;
    anyhow::ensure!(body == json!({"error": "File not found", "code": "FILE_NOT_FOUND"}), "{body}");
    Ok(body)
}

#[tokio::test]
#[ntest::timeout(120000)]
#[serial]
async fn test_shared_file_download_follows_owner_select_policy() -> anyhow::Result<()> {
    let server = start_http_test_server().await?;
    let result = async {
        let namespace = format!("file_rls_{}", unique_suffix());
        let owned = format!("owned_{}", unique_suffix());
        let locked = format!("locked_{}", unique_suffix());
        let either = format!("either_{}", unique_suffix());
        must_sql(&server, &format!("CREATE SCHEMA IF NOT EXISTS {namespace}")).await?;

        let (alice_auth, alice_id) =
            create_user_auth_header_with_id(&server, "alice", "test123", &Role::User).await?;
        let (bob_auth, _bob_id) =
            create_user_auth_header_with_id(&server, "bob", "test123", &Role::User).await?;
        let (service_auth, service_id) =
            create_user_auth_header_with_id(&server, "svc", "test123", &Role::Service).await?;
        let (dba_auth, _dba_id) =
            create_user_auth_header_with_id(&server, "dba", "test123", &Role::Dba).await?;

        must_sql(
            &server,
            &format!(
                "CREATE TABLE {namespace}.{owned} (id BIGINT PRIMARY KEY, owner_id TEXT, doc \
                 FILE) WITH (TYPE='SHARED')"
            ),
        )
        .await?;
        must_sql(
            &server,
            &format!(
                "CREATE POLICY owned_insert ON {namespace}.{owned} FOR INSERT TO user WITH CHECK \
                 (owner_id = CURRENT_USER)"
            ),
        )
        .await?;
        must_sql(
            &server,
            &format!(
                "CREATE POLICY owned_select ON {namespace}.{owned} FOR SELECT TO user USING \
                 (owner_id = CURRENT_USER)"
            ),
        )
        .await?;

        let upload = execute_sql_multipart(
            &server,
            &alice_auth,
            &format!(
                "INSERT INTO {namespace}.{owned} (id, owner_id, doc) VALUES (1, '{alice_id}', \
                 FILE(\"doc\"))"
            ),
            vec![("doc", "alice.txt", b"alice-bytes", "text/plain")],
        )
        .await?;
        assert_eq!(upload.status, ResponseStatus::Success, "{:?}", upload.error);

        let alice_ref = file_ref_at(
            &server,
            &alice_auth,
            &format!("SELECT doc FROM {namespace}.{owned} WHERE id = 1"),
            "doc",
        )
        .await?;
        let alice_url = file_url(&server, &namespace, &owned, &alice_ref);
        let alice_download = get_with_auth(&alice_url, Some(&alice_auth)).await?;
        assert_eq!(alice_download.status(), reqwest::StatusCode::OK);
        assert_eq!(alice_download.bytes().await?, b"alice-bytes".as_slice());

        let missing_url =
            format!("{}/v1/files/{namespace}/{owned}/f0001/missing-file.bin", server.base_url());
        let hidden = assert_missing_file(&alice_url, &bob_auth).await?;
        let missing = assert_missing_file(&missing_url, &bob_auth).await?;
        assert_eq!(hidden, missing, "a hidden FileRef must match a missing file");

        let service_upload = execute_sql_multipart(
            &server,
            &server.bearer_auth_header("root")?,
            &format!(
                "INSERT INTO {namespace}.{owned} (id, owner_id, doc) VALUES (2, '{service_id}', \
                 FILE(\"doc\"))"
            ),
            vec![("doc", "svc.txt", b"svc-bytes", "text/plain")],
        )
        .await?;
        assert_eq!(service_upload.status, ResponseStatus::Success, "{:?}", service_upload.error);
        let service_ref = file_ref_at(
            &server,
            &server.bearer_auth_header("root")?,
            &format!("SELECT doc FROM {namespace}.{owned} WHERE id = 2"),
            "doc",
        )
        .await?;
        assert_missing_file(&file_url(&server, &namespace, &owned, &service_ref), &service_auth)
            .await?;

        must_sql(
            &server,
            &format!(
                "CREATE TABLE {namespace}.{locked} (id BIGINT PRIMARY KEY, owner_id TEXT, doc \
                 FILE) WITH (TYPE='SHARED')"
            ),
        )
        .await?;
        must_sql(
            &server,
            &format!(
                "CREATE POLICY locked_insert ON {namespace}.{locked} FOR INSERT TO user WITH \
                 CHECK (owner_id = CURRENT_USER)"
            ),
        )
        .await?;
        let locked_upload = execute_sql_multipart(
            &server,
            &alice_auth,
            &format!(
                "INSERT INTO {namespace}.{locked} (id, owner_id, doc) VALUES (1, '{alice_id}', \
                 FILE(\"doc\"))"
            ),
            vec![("doc", "locked.txt", b"locked-bytes", "text/plain")],
        )
        .await?;
        assert_eq!(locked_upload.status, ResponseStatus::Success, "{:?}", locked_upload.error);
        let locked_ref = file_ref_at(
            &server,
            &dba_auth,
            &format!("SELECT doc FROM {namespace}.{locked} WHERE id = 1"),
            "doc",
        )
        .await?;
        let locked_url = file_url(&server, &namespace, &locked, &locked_ref);
        assert_missing_file(&locked_url, &alice_auth).await?;
        let dba_download = get_with_auth(&locked_url, Some(&dba_auth)).await?;
        assert_eq!(dba_download.status(), reqwest::StatusCode::OK);
        assert_eq!(dba_download.bytes().await?, b"locked-bytes".as_slice());

        must_sql(
            &server,
            &format!(
                "CREATE TABLE {namespace}.svc_only (id BIGINT PRIMARY KEY, doc FILE) WITH \
                 (TYPE='SHARED')"
            ),
        )
        .await?;
        must_sql(
            &server,
            &format!(
                "CREATE POLICY svc_read ON {namespace}.svc_only FOR SELECT TO service USING (true)"
            ),
        )
        .await?;
        let svc_only_upload = execute_sql_multipart(
            &server,
            &server.bearer_auth_header("root")?,
            &format!("INSERT INTO {namespace}.svc_only (id, doc) VALUES (1, FILE(\"doc\"))"),
            vec![("doc", "only.txt", b"service-only", "text/plain")],
        )
        .await?;
        assert_eq!(svc_only_upload.status, ResponseStatus::Success, "{:?}", svc_only_upload.error);
        let svc_only_ref = file_ref_at(
            &server,
            &service_auth,
            &format!("SELECT doc FROM {namespace}.svc_only WHERE id = 1"),
            "doc",
        )
        .await?;
        let svc_only_url = file_url(&server, &namespace, "svc_only", &svc_only_ref);
        let service_download = get_with_auth(&svc_only_url, Some(&service_auth)).await?;
        assert_eq!(service_download.status(), reqwest::StatusCode::OK);
        assert_eq!(service_download.bytes().await?, b"service-only".as_slice());
        assert_missing_file(&svc_only_url, &alice_auth).await?;

        must_sql(
            &server,
            &format!(
                "CREATE TABLE {namespace}.{either} (id BIGINT PRIMARY KEY, left_doc FILE, \
                 right_doc FILE) WITH (TYPE='SHARED')"
            ),
        )
        .await?;
        must_sql(
            &server,
            &format!(
                "CREATE POLICY either_select ON {namespace}.{either} FOR SELECT TO user USING \
                 (true)"
            ),
        )
        .await?;
        let either_upload = execute_sql_multipart(
            &server,
            &server.bearer_auth_header("root")?,
            &format!(
                "INSERT INTO {namespace}.{either} (id, left_doc, right_doc) VALUES (1, \
                 FILE(\"left\"), FILE(\"right\"))"
            ),
            vec![
                ("left", "left.txt", b"left-bytes", "text/plain"),
                ("right", "right.txt", b"right-bytes", "text/plain"),
            ],
        )
        .await?;
        assert_eq!(either_upload.status, ResponseStatus::Success, "{:?}", either_upload.error);
        let left_ref = file_ref_at(
            &server,
            &alice_auth,
            &format!("SELECT left_doc, right_doc FROM {namespace}.{either} WHERE id = 1"),
            "left_doc",
        )
        .await?;
        let right_ref = file_ref_at(
            &server,
            &alice_auth,
            &format!("SELECT left_doc, right_doc FROM {namespace}.{either} WHERE id = 1"),
            "right_doc",
        )
        .await?;
        let left_download =
            get_with_auth(&file_url(&server, &namespace, &either, &left_ref), Some(&alice_auth))
                .await?;
        assert_eq!(left_download.status(), reqwest::StatusCode::OK);
        assert_eq!(left_download.bytes().await?, b"left-bytes".as_slice());
        let right_download =
            get_with_auth(&file_url(&server, &namespace, &either, &right_ref), Some(&bob_auth))
                .await?;
        assert_eq!(right_download.status(), reqwest::StatusCode::OK);
        assert_eq!(right_download.bytes().await?, b"right-bytes".as_slice());
        assert_missing_file(
            &format!(
                "{}/v1/files/{namespace}/{either}/f0001/not-a-minted-file.bin",
                server.base_url()
            ),
            &alice_auth,
        )
        .await?;

        let traversal = get_with_auth(
            &format!("{}/v1/files/{namespace}/{either}/f00..1/file.bin", server.base_url()),
            Some(&alice_auth),
        )
        .await?;
        assert_eq!(traversal.status(), reqwest::StatusCode::BAD_REQUEST);
        let slashed = get_with_auth(
            &format!("{}/v1/files/{namespace}/{either}/f0001/a%2F..b.bin", server.base_url()),
            Some(&alice_auth),
        )
        .await?;
        assert!(
            matches!(
                slashed.status(),
                reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::NOT_FOUND
            ),
            "slash in a stored name must not return the blob, got {}",
            slashed.status()
        );
        assert_missing_file(
            &format!("{}/v1/files/{namespace}/{either}/sub/file.bin", server.base_url()),
            &alice_auth,
        )
        .await?;
        let anonymous = get_with_auth(&alice_url, None).await?;
        assert_eq!(anonymous.status(), reqwest::StatusCode::UNAUTHORIZED);

        let _ = server.execute_sql(&format!("DROP SCHEMA IF EXISTS {namespace} CASCADE")).await;
        Ok(())
    }
    .await;
    server.shutdown().await;
    result
}

#[tokio::test]
#[ntest::timeout(120000)]
#[serial]
async fn test_shared_file_download_conversation_membership() -> anyhow::Result<()> {
    let server = start_http_test_server().await?;
    let result = async {
        let namespace = format!("chat_{}", unique_suffix());
        let messages = format!("{namespace}.messages");
        let members = format!("{namespace}.members");
        must_sql(&server, &format!("CREATE SCHEMA IF NOT EXISTS {namespace}")).await?;
        must_sql(
            &server,
            &format!(
                "CREATE TABLE {messages} (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL,
                    attachment FILE
                ) WITH (TYPE='SHARED')"
            ),
        )
        .await?;
        must_sql(
            &server,
            &format!(
                "CREATE TABLE {members} (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    conversation_id TEXT NOT NULL,
                    kind TEXT NOT NULL
                ) WITH (TYPE='SHARED')"
            ),
        )
        .await?;

        let (alice_auth, alice_id) =
            create_user_auth_header_with_id(&server, "alice", "test123", &Role::User).await?;
        let (bob_auth, bob_id) =
            create_user_auth_header_with_id(&server, "bob", "test123", &Role::User).await?;
        let (service_auth, _service_id) =
            create_user_auth_header_with_id(&server, "svc", "test123", &Role::Service).await?;
        let root_auth = server.bearer_auth_header("root")?;

        must_sql(
            &server,
            &format!(
                "INSERT INTO {members} (id, user_id, conversation_id, kind) VALUES ('m-alice', \
                 '{alice_id}', 'conv-a', 'shared'), ('m-alice-private', '{alice_id}', 'conv-p', \
                 'private'), ('m-bob', '{bob_id}', 'conv-b', 'shared')"
            ),
        )
        .await?;
        must_sql(
            &server,
            &format!(
                "CREATE POLICY conversation_member_read ON {messages} FOR SELECT TO user USING (
                    conversation_id IN (
                        SELECT conversation_id FROM {members}
                        WHERE user_id = CURRENT_USER AND kind = 'shared'
                    )
                )"
            ),
        )
        .await?;

        for (id, conversation, field, filename, bytes) in [
            ("msg-a", "conv-a", "file_a", "a.txt", &b"bytes-a"[..]),
            ("msg-a2", "conv-a", "file_a2", "a2.txt", &b"bytes-a2"[..]),
            ("msg-private", "conv-p", "file_p", "p.txt", &b"bytes-private"[..]),
            ("msg-b", "conv-b", "file_b", "b.txt", &b"bytes-b"[..]),
        ] {
            let upload = execute_sql_multipart(
                &server,
                &root_auth,
                &format!(
                    "INSERT INTO {messages} (id, conversation_id, attachment) VALUES ('{id}', \
                     '{conversation}', FILE(\"{field}\"))"
                ),
                vec![(field, filename, bytes, "text/plain")],
            )
            .await?;
            assert_eq!(upload.status, ResponseStatus::Success, "{id}: {:?}", upload.error);
        }

        let file_for = |id: &'static str| {
            let server = &server;
            let root_auth = root_auth.clone();
            let messages = messages.clone();
            async move {
                file_ref_at(
                    server,
                    &root_auth,
                    &format!("SELECT attachment FROM {messages} WHERE id = '{id}'"),
                    "attachment",
                )
                .await
            }
        };
        let ref_a = file_for("msg-a").await?;
        let ref_a2 = file_for("msg-a2").await?;
        let ref_private = file_for("msg-private").await?;
        let ref_b = file_for("msg-b").await?;
        let url_a = file_url(&server, &namespace, "messages", &ref_a);
        let url_a2 = file_url(&server, &namespace, "messages", &ref_a2);
        let url_private = file_url(&server, &namespace, "messages", &ref_private);
        let url_b = file_url(&server, &namespace, "messages", &ref_b);

        let alice_a = get_with_auth(&url_a, Some(&alice_auth)).await?;
        assert_eq!(alice_a.status(), reqwest::StatusCode::OK);
        assert_eq!(alice_a.bytes().await?, b"bytes-a".as_slice());
        assert_missing_file(&url_a, &bob_auth).await?;
        assert_missing_file(&url_b, &alice_auth).await?;
        assert_missing_file(&url_private, &alice_auth).await?;
        assert_missing_file(&url_a, &service_auth).await?;

        must_sql(&server, &format!("UPDATE {messages} SET attachment = NULL WHERE id = 'msg-a'"))
            .await?;
        assert_missing_file(&url_a, &alice_auth).await?;

        must_sql(&server, &format!("DELETE FROM {members} WHERE id = 'm-alice'")).await?;
        assert_missing_file(&url_a2, &alice_auth).await?;

        let _ = server.execute_sql(&format!("DROP SCHEMA IF EXISTS {namespace} CASCADE")).await;
        Ok(())
    }
    .await;
    server.shutdown().await;
    result
}
