//! PostgreSQL JDBC + HikariCP e2e against a live KalamDB pgwire listener.
//!
//! Requires JDK (`java` on PATH) and the same env as the catalog tests:
//! `KALAMDB_PGWIRE_HOST`, `KALAMDB_PGWIRE_PORT`, optional user/password.
//!
//! Jars are downloaded once into `backend/target/jdbc-jars`.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const POSTGRESQL_JAR: MavenJar = MavenJar {
    group:   "org/postgresql",
    artifact: "postgresql",
    version: "42.7.5",
};
const HIKARI_JAR: MavenJar = MavenJar {
    group:   "com/zaxxer",
    artifact: "HikariCP",
    version: "5.1.0",
};
const SLF4J_API_JAR: MavenJar = MavenJar {
    group:   "org/slf4j",
    artifact: "slf4j-api",
    version: "1.7.36",
};
const SLF4J_NOP_JAR: MavenJar = MavenJar {
    group:   "org/slf4j",
    artifact: "slf4j-nop",
    version: "1.7.36",
};

struct MavenJar {
    group:    &'static str,
    artifact: &'static str,
    version:  &'static str,
}

impl MavenJar {
    fn file_name(&self) -> String {
        format!("{}-{}.jar", self.artifact, self.version)
    }

    fn url(&self) -> String {
        format!(
            "https://repo1.maven.org/maven2/{}/{}/{}/{}",
            self.group,
            self.artifact,
            self.version,
            self.file_name()
        )
    }
}

fn pgwire_env() -> Option<(String, u16, String, String)> {
    let host = env::var("KALAMDB_PGWIRE_HOST").ok()?;
    let port: u16 = env::var("KALAMDB_PGWIRE_PORT").ok()?.parse().ok()?;
    let user = env::var("KALAMDB_PGWIRE_USER").unwrap_or_else(|_| "root".to_string());
    let password = env::var("KALAMDB_PGWIRE_PASSWORD").unwrap_or_default();
    Some((host, port, user, password))
}

fn java_bin() -> Option<PathBuf> {
    which("java")
}

fn which(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
    })
}

fn jar_cache_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/jdbc-jars")
}

fn smoke_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/pgwire_catalog/jdbc/JdbcPgwireSmoke.java")
}

async fn ensure_jar(jar: &MavenJar, cache: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(cache).map_err(|error| format!("create jar cache: {error}"))?;
    let dest = cache.join(jar.file_name());
    if dest.is_file() && dest.metadata().map(|meta| meta.len() > 0).unwrap_or(false) {
        return Ok(dest);
    }

    let url = jar.url();
    let bytes = reqwest::get(&url)
        .await
        .map_err(|error| format!("download {}: {error}", jar.file_name()))?
        .error_for_status()
        .map_err(|error| format!("download {}: {error}", jar.file_name()))?
        .bytes()
        .await
        .map_err(|error| format!("download body {}: {error}", jar.file_name()))?;
    fs::write(&dest, &bytes).map_err(|error| format!("write {}: {error}", jar.file_name()))?;
    Ok(dest)
}

fn classpath(jars: &[PathBuf]) -> String {
    let sep = if cfg!(windows) { ";" } else { ":" };
    jars.iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(sep)
}

fn require_marker(stdout: &str, marker: &str) {
    assert!(
        stdout.contains(marker),
        "JDBC smoke stdout missing `{marker}`:\n{stdout}"
    );
}

/// HikariCP pool init + JDBC query/DML against KalamDB pgwire (the original JDBC failure).
#[tokio::test]
#[ignore = "requires postgres wire listener, JDK, and network for Maven jars; see \
            tests/pgwire_catalog/jdbc"]
#[ntest::timeout(120000)]
async fn jdbc_hikari_pool_connects_and_queries() {
    let Some((host, port, user, password)) = pgwire_env() else {
        panic!("Set KALAMDB_PGWIRE_HOST and KALAMDB_PGWIRE_PORT to run against a live server");
    };
    let Some(java) = java_bin() else {
        if env::var("KALAMDB_PGWIRE_REQUIRE_JDBC").is_ok() {
            panic!("java is required when KALAMDB_PGWIRE_REQUIRE_JDBC is set");
        }
        eprintln!("skipping JDBC e2e; install a JDK (`java` on PATH) or set KALAMDB_PGWIRE_REQUIRE_JDBC=1");
        return;
    };

    let cache = jar_cache_dir();
    let jars = [
        ensure_jar(&POSTGRESQL_JAR, &cache).await.expect("postgresql jar"),
        ensure_jar(&HIKARI_JAR, &cache).await.expect("HikariCP jar"),
        ensure_jar(&SLF4J_API_JAR, &cache).await.expect("slf4j-api jar"),
        ensure_jar(&SLF4J_NOP_JAR, &cache).await.expect("slf4j-nop jar"),
    ];
    let source = smoke_source();
    assert!(source.is_file(), "missing JDBC smoke source at {}", source.display());

    let jdbc_url = format!("jdbc:postgresql://{host}:{port}/kalam?sslmode=disable");
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let namespace = "jdbc_e2e";
    let table = format!("smoke_{suffix}");

    let output = Command::new(java)
        .arg("-Dfile.encoding=UTF-8")
        .arg("-cp")
        .arg(classpath(&jars))
        .arg(&source)
        .arg(&jdbc_url)
        .arg(&user)
        .arg(&password)
        .arg(namespace)
        .arg(&table)
        .output()
        .expect("spawn java JDBC smoke");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "JDBC/Hikari smoke failed (status {:?})\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );

    require_marker(&stdout, "driver_manager_ok");
    require_marker(&stdout, "hikari_pool_ok");
    require_marker(&stdout, "isolation=2");
    require_marker(&stdout, "select_1_ok");
    require_marker(&stdout, "prepared_select_ok");
    require_marker(&stdout, "prepared_param_ok");
    require_marker(&stdout, "show_isolation_ok");
    require_marker(&stdout, "dml_roundtrip_ok");
    require_marker(&stdout, "information_schema_tables_ok");
    require_marker(&stdout, "get_catalogs_ok");
    require_marker(&stdout, "get_schemas_ok");
    require_marker(&stdout, "get_table_types_ok");
    require_marker(&stdout, "get_tables_ok");
    require_marker(&stdout, "get_columns_ok");
    require_marker(&stdout, "get_primary_keys_ok");
    require_marker(&stdout, "jdbc_pgwire_smoke_ok");
}
