#![cfg(unix)]

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use tempfile::TempDir;

const FIXTURE_VERSION: &str = "9.9.9-test.1";
const UPDATED_FIXTURE_OUTPUT: &str = "updated-cli-fixture";
const SERVER_RELEASE_BASE_URL_ENV: &str = "KALAMDB_SERVER_RELEASE_BASE_URL";

#[test]
fn test_cli_update_command_replaces_binary() {
    let temp_dir = TempDir::new().expect("temp dir");
    let platform = current_platform();
    let cli_archive_name = format!("kalamcli-{}-{}.tar.gz", FIXTURE_VERSION, platform);
    let server_archive_name = format!("kalamdb-server-{}-{}.tar.gz", FIXTURE_VERSION, platform);
    let cli_archive_bytes = build_cli_release_archive(FIXTURE_VERSION, &platform);
    let server_archive_bytes = build_server_release_archive();
    let checksums = format!(
        "{}  {}\n{}  {}\n",
        hex::encode(Sha256::digest(&cli_archive_bytes)),
        cli_archive_name,
        hex::encode(Sha256::digest(&server_archive_bytes)),
        server_archive_name,
    );
    let mut releases = HashMap::new();
    releases.insert(
        format!("/releases/download/v{}/{}", FIXTURE_VERSION, cli_archive_name),
        cli_archive_bytes,
    );
    releases.insert(
        format!("/releases/download/v{}/{}", FIXTURE_VERSION, server_archive_name),
        server_archive_bytes,
    );
    let server = MockReleaseServer::spawn(releases, checksums.into_bytes());

    let binary_path = temp_dir.path().join("kalam-under-test");
    fs::copy(crate::common::kalam_bin(), &binary_path).expect("copy kalam binary");

    let output = Command::new(&binary_path)
        .arg("--no-color")
        .arg("--no-spinner")
        .arg("update")
        .arg("--version")
        .arg(FIXTURE_VERSION)
        .arg("--force")
        .env("KALAM_CLI_RELEASE_BASE_URL", server.base_url())
        .env(SERVER_RELEASE_BASE_URL_ENV, server.base_url())
        .env("NO_PROXY", "127.0.0.1,localhost,::1")
        .env("no_proxy", "127.0.0.1,localhost,::1")
        .output()
        .expect("run kalam update");

    assert!(
        output.status.success(),
        "update should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("Installed kalam {}", FIXTURE_VERSION)),
        "update stdout should confirm installation\nstdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let updated = Command::new(&binary_path)
        .arg("--version")
        .output()
        .expect("run updated fixture");
    assert!(updated.status.success(), "updated fixture should run");
    assert_eq!(String::from_utf8_lossy(&updated.stdout).trim(), UPDATED_FIXTURE_OUTPUT);
}

fn current_platform() -> String {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        other => panic!("unsupported test operating system: {}", other),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => panic!("unsupported test architecture: {}", other),
    };
    format!("{}-{}", os, arch)
}

fn build_cli_release_archive(version: &str, platform: &str) -> Vec<u8> {
    let script = format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", UPDATED_FIXTURE_OUTPUT);
    let entry_path = format!("kalamcli-{}-{}", version, platform);
    build_tar_gz_archive(&entry_path, script.as_bytes())
}

fn build_server_release_archive() -> Vec<u8> {
    let script = b"#!/bin/sh\nprintf '%s\\n' 'kalamdb-server-fixture'\n";
    build_tar_gz_archive("kalamdb-server", script)
}

fn build_tar_gz_archive(entry_path: &str, payload: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = Builder::new(&mut encoder);
        let mut header = Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, Path::new(entry_path), payload)
            .expect("append fixture binary");
        archive.finish().expect("finish tar archive");
    }

    encoder.finish().expect("finish gzip archive")
}

struct MockReleaseServer {
    base_url: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockReleaseServer {
    fn spawn(releases: HashMap<String, Vec<u8>>, checksum_body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind release server");
        listener.set_nonblocking(true).expect("set release server nonblocking");
        let base_url = format!(
            "http://{}/releases/download/v{}",
            listener.local_addr().expect("release server addr"),
            FIXTURE_VERSION
        );
        let checksum_path = format!("/releases/download/v{}/SHA256SUMS", FIXTURE_VERSION);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_signal = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            while !stop_signal.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        handle_request(stream, &releases, &checksum_path, &checksum_body)
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    },
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url,
            stop,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for MockReleaseServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(
            self.base_url
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or_default(),
        );
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_request(
    mut stream: TcpStream,
    releases: &HashMap<String, Vec<u8>>,
    checksum_path: &str,
    checksum_body: &[u8],
) {
    let mut buffer = [0u8; 4096];
    let Ok(read) = stream.read(&mut buffer) else {
        return;
    };
    if read == 0 {
        return;
    }

    let request = String::from_utf8_lossy(&buffer[..read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let (status, body) = if path == checksum_path {
        ("200 OK", checksum_body)
    } else if let Some(body) = releases.get(path) {
        ("200 OK", body.as_slice())
    } else {
        ("404 Not Found", b"not found".as_slice())
    };

    let response = format!(
        "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
