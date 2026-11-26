#!/usr/bin/env python3
"""Rehearse a stopped standalone upgrade, rollback and logical restore."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import tarfile
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener
import uuid
import zipfile

HERE = Path(__file__).resolve().parents[1]
PRODUCT = "kalamdb/KalamDB"
ASSET_RE = re.compile(r"kalamdb-server-([0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.+-]+)?)-linux-x86_64")
STABLE_RE = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+")
SHA_RE = re.compile(r"[0-9a-f]{40}")
DIGEST_RE = re.compile(r"[0-9a-f]{64}")
OPENER = build_opener(ProxyHandler({}))
CHECKS = ("direct_upgrade", "durable_restart", "rollback", "logical_restore")


def need(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def github(endpoint):
    return json.loads(subprocess.check_output(["gh", "api", endpoint], text=True))


def download(url, path, authenticated=False):
    cmd = ["curl", "--proto", "=https", "--proto-redir", "=https", "-fsSL",
           "--retry", "2", "--max-time", "150", "--output", str(path)]
    if authenticated:
        # curl strips Authorization on a cross-origin redirect. Do not enable --location-trusted.
        cmd.extend(["--header", "Authorization: Bearer " + os.environ["GH_TOKEN"]])
    result = subprocess.run(cmd + [url], check=False, capture_output=True, text=True)
    need(result.returncode == 0, f"Download failed for {url}: {result.stderr.strip()}")


def checksum_for(text, filename):
    matches = []
    for row in text.splitlines():
        fields = row.split(maxsplit=1)
        if len(fields) == 2 and fields[1].lstrip("*") == filename:
            matches.append(fields[0].lower())
    need(len(matches) == 1 and DIGEST_RE.fullmatch(matches[0]),
         f"SHA256SUMS must contain exactly one valid checksum for {filename}")
    return matches[0]


def copy_member(src, dst):
    with src, dst.open("xb") as out:
        shutil.copyfileobj(src, out)
    dst.chmod(0o700)


def load_baseline(tmp, tag, report):
    need(not tag or STABLE_RE.fullmatch(tag), "previous_tag must look like v0.8.0")
    endpoint = f"repos/{PRODUCT}/releases/" + (f"tags/{tag}" if tag else "latest")
    release = github(endpoint)
    tag = release["tag_name"]
    need(STABLE_RE.fullmatch(tag) and not release["draft"] and not release["prerelease"]
         and release["published_at"], f"{tag} is not a published stable baseline")
    assets = {asset["name"]: asset for asset in release["assets"]}
    name = f"kalamdb-server-{tag[1:]}-linux-x86_64.tar.gz"
    expected_prefix = f"https://github.com/{PRODUCT}/releases/download/{tag}/"
    for filename in (name, "SHA256SUMS"):
        url = assets[filename]["browser_download_url"]
        need(url == expected_prefix + filename, f"Unexpected release asset URL for {filename}")
        download(url, tmp / filename)
    digest = checksum_for((tmp / "SHA256SUMS").read_text(), name)
    need(sha256(tmp / name) == digest, f"Baseline asset checksum mismatch: {name}")
    report.update(previous_tag=tag, previous_asset_sha256=digest)
    binary = tmp / "baseline-server"
    with tarfile.open(tmp / name, "r:gz") as archive:
        member = archive.getmember(name.removesuffix(".tar.gz"))
        need(member.isfile(), "Baseline archive server is not a regular file")
        copy_member(archive.extractfile(member), binary)
    config = tmp / "baseline.toml"
    download(f"https://raw.githubusercontent.com/{PRODUCT}/{tag}/server.toml", config)
    return binary, config


def run_matches(run, candidate_sha):
    return (
        run.get("head_sha") == candidate_sha
        and run.get("path", "").split("@", 1)[0] == ".github/workflows/release.yml"
        and run.get("head_repository", {}).get("full_name", "").lower() == PRODUCT.lower()
        and run.get("status") == "completed"
        and run.get("conclusion") == "success"
    )


def load_candidate(tmp, sha, run_id, report):
    need(SHA_RE.fullmatch(sha), "Candidate SHA must be a full Git commit SHA")
    if run_id:
        need(re.fullmatch(r"[1-9][0-9]{0,19}", run_id), "candidate_run_id must be an Actions run ID")
        run = github(f"repos/{PRODUCT}/actions/runs/{run_id}")
        need(run_matches(run, sha), f"Release run {run_id} is not successful for candidate {sha}")
    else:
        result = github(f"repos/{PRODUCT}/actions/runs?head_sha={sha}&status=success&per_page=100")
        runs = sorted((r for r in result["workflow_runs"] if run_matches(r, sha)),
                      key=lambda r: r["id"], reverse=True)
        need(runs, f"No successful Release run for {sha}. Run Release on this ref with all"
                   " available publication switches disabled, then rerun this gate.")
        run = runs[0]
    run_id = run["id"]
    report.update(candidate_run_id=run_id, candidate_run_url=run["html_url"])
    artifacts = github(f"repos/{PRODUCT}/actions/runs/{run_id}/artifacts?per_page=100")["artifacts"]
    matches = [a for a in artifacts if a["name"] == "dist-linux-x86_64" and not a["expired"]]
    need(len(matches) == 1, "Expected exactly one unexpired dist-linux-x86_64 Release artifact")
    artifact = matches[0]
    digest = (artifact.get("digest") or "").removeprefix("sha256:")
    need(DIGEST_RE.fullmatch(digest), "Candidate artifact has no valid SHA-256 digest")
    report.update(candidate_artifact_id=artifact["id"], candidate_artifact_sha256=digest)
    path = tmp / "candidate.zip"
    download(f"https://api.github.com/repos/{PRODUCT}/actions/artifacts/{artifact['id']}/zip",
             path, authenticated=True)
    need(sha256(path) == digest, "Downloaded candidate workflow artifact digest mismatch")
    binary = tmp / "candidate-server"
    with zipfile.ZipFile(path) as archive:
        members = [m for m in archive.infolist() if ASSET_RE.fullmatch(m.filename)]
        need(len(members) == 1, "Expected exactly one Linux x86_64 server binary in candidate ZIP")
        copy_member(archive.open(members[0]), binary)
    report["candidate_binary_sha256"] = sha256(binary)
    return binary

def config_for(path, target, data, port, rpc):
    settings = {
        "server": {
            "host": '"127.0.0.1"', "port": str(port), "workers": "2", "public_origin": '""',
        },
        "storage": {"data_path": json.dumps(str(data))},
        "logging": {"level": '"info"', "logs_path": json.dumps(str(target.parent / "logs"))},
        "logging.otlp": {"enabled": "false"},
        "postgres_wire": {"enabled": "false"},
        "pg_extension": {"enabled": "false"},
        "auth.oidc": {"enabled": "false"},
        "cluster": {
            "node_id": "1", "api_addr": f'"127.0.0.1:{port}"', "rpc_addr": f'"127.0.0.1:{rpc}"',
            "user_shards": "1", "shared_shards": "1", "peers": "[]",
        },
        "flush": {"check_interval_seconds": "0"},
    }
    text = path.read_text()
    for section, keys in settings.items():
        head = re.compile(r"(?m)^" + re.escape(f"[{section}]") + r"[ \t]*$")
        match = head.search(text)
        if not match:
            text += f"\n[{section}]\n"
            match = head.search(text)
        next_head = re.search(r"(?m)^\[", text[match.end():])
        end = match.end() + next_head.start() if next_head else len(text)
        body = text[match.end():end]
        for key, value in keys.items():
            old = re.compile(r"(?m)^" + key + r"[ \t]*=[^\n]*")
            need(key != "peers" or not re.search(r"(?m)^peers\s*=\s*\[\s*$", body),
                 "Unexpected nonempty multiline peers in release config")
            replacement = f"{key} = {value}"
            body = old.sub(lambda _: replacement, body) if old.search(body) else body + f"\n{replacement}\n"
        text = text[:match.end()] + body + text[end:]
    target.write_text(text)


def ports():
    with socket.socket() as http, socket.socket() as rpc:
        for sock in (http, rpc):
            sock.bind(("127.0.0.1", 0))
        return http.getsockname()[1], rpc.getsockname()[1]


def req(port, path, body=None, token=None, content_type="application/json"):
    headers = {"Content-Type": content_type}
    if token:
        headers["Authorization"] = "Bearer " + token
    data = json.dumps(body).encode() if isinstance(body, dict) else body
    request = Request(f"http://127.0.0.1:{port}{path}", data=data, headers=headers)
    try:
        with OPENER.open(request, timeout=8) as response:
            return response.status, response.read()
    except HTTPError as error:
        return error.code, error.read()


def api(port, path, body=None, token=None, content_type="application/json"):
    status, raw = req(port, path, body, token, content_type)
    need(200 <= status < 300, f"{path} returned HTTP {status}")
    return json.loads(raw)


def login(port, name, password):
    result = api(port, "/v1/api/auth/login", {"user": name, "password": password})
    token = result.get("access_token") or result.get("token") or result.get("accessToken")
    need(token, f"Login response omitted token for {name}")
    return token


def sql(port, token, query):
    try:
        result = api(port, "/v1/api/sql", {"sql": query}, token)
    except RuntimeError as error:
        verb = query.split(maxsplit=1)[0]
        raise RuntimeError(f"{verb} failed: {error}") from None
    need(result.get("status") == "success" and not any(row.get("error") for row in result.get("results", [])),
         f"SQL returned failure for {query.split(maxsplit=1)[0]}")
    return result["results"]


def records(results):
    out = []
    for result in results:
        fields = [col["name"] if isinstance(col, dict) else col for col in result.get("schema", [])]
        for row in result.get("rows") or ():
            out.append(row if isinstance(row, dict) else dict(zip(fields, row)))
    return out


def until(probe, timeout, label):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        result = probe()
        if result:
            return result
        time.sleep(0.25)
    raise TimeoutError(f"Timed out waiting for {label}")


def job(port, token, job_id):
    need(job_id and re.fullmatch(r"[A-Za-z0-9_-]+", job_id), "Transfer returned invalid job ID")
    def poll():
        rows = records(sql(port, token, f"SELECT status FROM system.jobs WHERE job_id = '{job_id}'"))
        if rows:
            state = rows[0]["status"]
            need(state not in ("failed", "cancelled", "canceled"), f"Job {job_id} ended as {state}")
            return state == "completed"
        return False
    until(poll, 90, job_id)


def flush(port, token, table):
    message = sql(port, token, f"STORAGE FLUSH TABLE qa.{table}")[0].get("message", "")
    match = re.search(r"Job ID: (FL-[A-Za-z0-9-]+)", message)
    need(match, "Storage flush response omitted job ID")
    job(port, token, match[1])


def export(port, root, table, scope, owner=None):
    fields = {"namespace_id": "qa", "table_name": table, "table_type": scope}
    if owner:
        fields["user_id"] = owner
    answer = api(port, "/v1/api/table-exports", fields, root)
    job_id = answer["job_id"]
    job(port, root, job_id)
    rows = records(sql(port, root, "SHOW EXPORT"))
    paths = [row.get("download_url") for row in rows if job_id in row.values()]
    need(len(paths) == 1 and paths[0].startswith("/v1/table-exports/"),
         f"Export download path missing for {job_id}")
    status, data = req(port, paths[0], token=root)
    need(status == 200, f"Export download failed for {job_id}: HTTP {status}")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        files = [name for name in archive.namelist() if name.startswith("data/") and name.endswith(".parquet")]
        need("kalamdb-table-export.json" in archive.namelist() and files, "Export is missing manifest or Parquet")
    return data


def import_archive(port, root, table, scope, owner, archive):
    fields = {"namespace_id": "qa", "table_name": table, "table_type": scope}
    if owner:
        fields["user_id"] = owner
    boundary = "KalamUpgrade" + uuid.uuid4().hex
    blob = b"".join(
        f'--{boundary}\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'.encode()
        for key, value in fields.items()
    )
    blob += (
        f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="fixture.zip"\r\n'
        "Content-Type: application/zip\r\n\r\n"
    ).encode() + archive + f"\r\n--{boundary}--\r\n".encode()
    answer = api(port, "/v1/api/table-imports", blob, root, "multipart/form-data; boundary=" + boundary)
    job(port, root, answer["job_id"])


class Rehearsal:
    def __init__(self, temp, evidence):
        self.temp = temp
        self.evidence = evidence
        self.processes = []
        self.password = "Fixture!" + secrets.token_hex(12)
        self.jwt_secret = secrets.token_hex(32)
        self.log_index = 0

    def launch(self, binary, config, data, name):
        self.log_index += 1
        port, rpc = ports()
        sandbox = self.temp / f"server-{self.log_index}"
        sandbox.mkdir()
        config_for(config, sandbox / "server.toml", data, port, rpc)
        # Do not expose the workflow's GH token to the server.
        env = {key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL") if key in os.environ}
        env.update(HOME=str(sandbox), KALAMDB_ROOT_PASSWORD=self.password, KALAMDB_JWT_SECRET=self.jwt_secret)
        log = self.evidence / f"{self.log_index:02d}-{name}.log"
        with log.open("wb") as output:
            process = subprocess.Popen([str(binary), "server.toml"], cwd=sandbox, env=env,
                                       stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT)
        self.processes.append(process)

        def ready():
            need(process.poll() is None, f"{name} exited with {process.returncode}; see {log.name}")
            try:
                return req(port, "/health")[0] == 200
            except (OSError, TimeoutError):
                return False

        try:
            until(ready, 45, name)
        except Exception:
            self.stop(process)
            raise
        return port, process

    @staticmethod
    def stop(process):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
                raise RuntimeError("Server did not stop cleanly")

    def clean(self):
        for process in reversed(self.processes):
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)

    def users(self, port):
        return {name: login(port, name, self.password) for name in ("root", "fixture_alice", "fixture_bob")}

    def schema(self, port):
        root = login(port, "root", self.password)
        for query in (
            "CREATE SCHEMA qa",
            "CREATE TABLE qa.shared_rows (id BIGINT PRIMARY KEY, note TEXT) WITH (TYPE='SHARED')",
            "CREATE TABLE qa.user_rows (id BIGINT PRIMARY KEY, note TEXT) WITH (TYPE='USER')",
            f"CREATE USER fixture_alice WITH PASSWORD '{self.password}' ROLE 'service'",
            f"CREATE USER fixture_bob WITH PASSWORD '{self.password}' ROLE 'service'",
        ):
            sql(port, root, query)
        return self.users(port)

    @staticmethod
    def snapshot(port, users):
        dataset = {}
        for name, table in (("root", "shared_rows"), ("fixture_alice", "user_rows"), ("fixture_bob", "user_rows")):
            rows = records(sql(port, users[name], f"SELECT id, note FROM qa.{table} ORDER BY id"))
            dataset[name] = [(int(row["id"]), row["note"]) for row in rows]
        return dataset

    def confirm(self, port, expected, label):
        users = self.users(port)
        actual = self.snapshot(port, users)
        need(actual == expected, f"{label}: expected {expected}, got {actual}")
        return users

    def exercise_write(self, port, expected, label):
        users = self.confirm(port, expected, label)
        sql(port, users["root"], "UPDATE qa.shared_rows SET note='after upgrade' WHERE id=1")
        sql(port, users["fixture_alice"], "INSERT INTO qa.user_rows VALUES (3,'alice after upgrade')")
        changed = {
            **expected,
            "root": [(1, "after upgrade"), *expected["root"][1:]],
            "fixture_alice": [*expected["fixture_alice"], (3, "alice after upgrade")],
        }
        self.confirm(port, changed, label + " after writes")
        return changed

    def run(self, baseline, baseline_config, candidate, candidate_config, report):
        source, backup = self.temp / "active-data", self.temp / "untouched-backup"
        logical = self.temp / "logical-next"
        expected = {
            "root": [(1, "shared cold"), (2, "shared updated"), (3, "shared hot")],
            "fixture_alice": [(1, "alice cold"), (2, "alice hot")],
            "fixture_bob": [(1, "bob cold"), (2, "bob hot")],
        }
        port, proc = self.launch(baseline, baseline_config, source, "baseline")
        people = self.schema(port)
        root, alice, bob = (people[k] for k in ("root", "fixture_alice", "fixture_bob"))
        for who, statement in (
            (root, "INSERT INTO qa.shared_rows VALUES (1,'shared cold'),(2,'shared old')"),
            (alice, "INSERT INTO qa.user_rows VALUES (1,'alice cold')"),
            (bob, "INSERT INTO qa.user_rows VALUES (1,'bob cold')"),
        ):
            sql(port, who, statement)
        for table in ("shared_rows", "user_rows"):
            flush(port, root, table)
        need(len(list((source / "storage").rglob("*.parquet"))) >= 3, "Fixture did not create cold Parquet")
        for who, statement in (
            (root, "UPDATE qa.shared_rows SET note='shared updated' WHERE id=2"),
            (root, "INSERT INTO qa.shared_rows VALUES (3,'shared hot')"),
            (alice, "INSERT INTO qa.user_rows VALUES (2,'alice hot')"),
            (bob, "INSERT INTO qa.user_rows VALUES (2,'bob hot')"),
        ):
            sql(port, who, statement)
        self.confirm(port, expected, "baseline")
        specs = (("shared_rows", "shared", None), ("user_rows", "user", "fixture_alice"),
                 ("user_rows", "user", "fixture_bob"))
        try:
            archives = [export(port, root, *spec) for spec in specs]
        except Exception as error:
            archives = None
            report["checks"]["logical_restore"] = "fail"
            report["errors"]["logical_restore"] = str(error)
            print(f"logical_restore export: {error}", flush=True)
        finally:
            self.stop(proc)

        # Retain the baseline untouched, and successively restore independent copies to the
        # original configured path: baseline local/Parquet storage may pin absolute paths.
        # No original copy remains active while either binary is under test.
        source.rename(backup)
        shutil.copytree(backup, source)

        def direct_check():
            port, proc = self.launch(candidate, candidate_config, source, "direct")
            try:
                changed = self.exercise_write(port, expected, "direct upgrade")
                report["checks"]["direct_upgrade"] = "pass"
            finally:
                self.stop(proc)
            port, proc = self.launch(candidate, candidate_config, source, "direct-restart")
            try:
                self.confirm(port, changed, "direct restart")
                report["checks"]["durable_restart"] = "pass"
            finally:
                self.stop(proc)

        def rollback_check():
            shutil.rmtree(source)
            shutil.copytree(backup, source)
            port, proc = self.launch(baseline, baseline_config, source, "untouched-backup")
            try:
                self.confirm(port, expected, "rollback from untouched baseline copy")
            finally:
                self.stop(proc)

        def logical_check():
            port, proc = self.launch(candidate, candidate_config, logical, "logical-restore")
            try:
                users = self.schema(port)
                for spec, archive in zip(specs, archives):
                    import_archive(port, users["root"], *spec, archive)
                changed = self.exercise_write(port, expected, "logical restore")
            finally:
                self.stop(proc)
            port, proc = self.launch(candidate, candidate_config, logical, "logical-restart")
            try:
                self.confirm(port, changed, "logical restart")
            finally:
                self.stop(proc)

        checks = [("direct_upgrade", direct_check), ("rollback", rollback_check)]
        if archives is not None:
            checks.append(("logical_restore", logical_check))
        for name, check in checks:
            try:
                check()
                if name != "direct_upgrade":
                    report["checks"][name] = "pass"
            except Exception as error:
                failed = "durable_restart" if name == "direct_upgrade" and report["checks"][name] == "pass" else name
                report["checks"][failed] = "fail"
                report["errors"][failed] = str(error)
                print(f"{failed}: {error}", flush=True)
        print(json.dumps(report["checks"], sort_keys=True), flush=True)

def version(binary):
    label = subprocess.check_output([str(binary), "--version"], text=True, timeout=8).strip()
    match = re.search(r"\bv(\d+)\.(\d+)\.(\d+)\b", label)
    need(match, "Cannot parse server version from version banner")
    return tuple(map(int, match.groups())), label


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path, default=Path("upgrade-compatibility-evidence"))
    parser.add_argument("--candidate-sha", default=os.environ.get("GITHUB_SHA", ""))
    parser.add_argument("--candidate-run-id", default=os.environ.get("CANDIDATE_RUN_ID", ""))
    parser.add_argument("--previous-tag", default=os.environ.get("PREVIOUS_TAG", ""))
    parser.add_argument("--candidate-config", type=Path, default=HERE / "server.toml")
    parser.add_argument("--require-logical-restore", action="store_true",
                        default=os.environ.get("REQUIRE_LOGICAL_RESTORE", "").lower() == "true")
    # Offline mode exercises the same fixture with independently verified, already unpacked binaries.
    parser.add_argument("--local-baseline-server", type=Path)
    parser.add_argument("--local-baseline-config", type=Path)
    parser.add_argument("--local-candidate-server", type=Path)
    args = parser.parse_args()
    local = (args.local_baseline_server, args.local_baseline_config, args.local_candidate_server)
    if any(local) and not all(local):
        parser.error("offline rehearsal needs both local server binaries and the baseline config")

    evidence = args.evidence_dir.resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    report = {
        "schema_version": 1,
        "source_mode": "local" if all(local) else "github_release",
        "status": "fail",
        "candidate_sha": args.candidate_sha,
        "checks": dict.fromkeys(CHECKS, "not_run"),
        "required_checks": list(CHECKS if args.require_logical_restore else CHECKS[:-1]),
        "errors": {},
    }
    try:
        with tempfile.TemporaryDirectory(prefix="kalam-upgrade-") as path:
            tmp = Path(path)
            if all(local):
                baseline, baseline_config, candidate = (arg.resolve() for arg in local)
                report.update(previous_tag=args.previous_tag or None, candidate_binary_sha256=sha256(candidate))
            else:
                need(os.environ.get("GITHUB_REPOSITORY", "").lower() == PRODUCT.lower(),
                     f"Artifact-mode workflow must run in {PRODUCT}")
                candidate = load_candidate(tmp, args.candidate_sha, args.candidate_run_id, report)
                baseline, baseline_config = load_baseline(tmp, args.previous_tag, report)
            old_version, old_label = version(baseline)
            new_version, new_label = version(candidate)
            report.update(baseline_version=old_label, candidate_version=new_label)
            if report["previous_tag"]:
                need(report["previous_tag"] == "v" + ".".join(map(str, old_version)),
                     "Baseline server reports a different version from its release tag")
            need(new_version > old_version, "Candidate must be newer than the baseline; select the prior stable tag")
            print(f"Baseline: {old_label}\nCandidate: {new_label}", flush=True)
            fixture = Rehearsal(tmp, evidence)
            try:
                fixture.run(baseline, baseline_config, candidate, args.candidate_config.resolve(), report)
            finally:
                fixture.clean()
            if all(report["checks"][k] == "pass" for k in report["required_checks"]):
                report["status"] = "pass"
    except Exception as error:
        report["errors"]["setup"] = str(error)
        print(f"Upgrade rehearsal failed: {error}", flush=True)
    finally:
        (evidence / "upgrade-compatibility.json").write_text(json.dumps(report, indent=2) + "\n")
        if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
            with Path(summary).open("a") as output:
                output.write("## Upgrade compatibility (standalone Linux x86_64)\n\n")
                output.write(f"- Overall: **{report['status'].upper()}**\n")
                output.write(f"- Candidate: `{report['candidate_sha']}`\n")
                output.write(f"- Baseline: `{report.get('previous_tag') or 'unresolved'}`\n")
                if report.get("candidate_run_url"):
                    output.write(f"- Candidate package: [Release run]({report['candidate_run_url']})\n")
                for name, state in report["checks"].items():
                    requirement = "" if name in report["required_checks"] else " (advisory)"
                    output.write(f"- {name}: **{state.upper()}**{requirement}\n")
    return 0 if report["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
