// Performance tests inside the e2e binary.
//
// `sqlite_speed` is the PGlite SQLite speed suite and runs with the rest of
// `cargo nextest run -p kalam-cli-e2e --test e2e`. The memory regression stays
// ignored because it needs an otherwise idle server.

#[path = "performance/sqlite_speed/mod.rs"]
mod sqlite_speed;
#[path = "performance/test_server_memory_regression.rs"]
mod test_server_memory_regression;
