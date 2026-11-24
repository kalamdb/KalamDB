# Standalone upgrade compatibility check

Use **Actions → Upgrade Compatibility → Run workflow** on the candidate branch.
The candidate is the exact commit shown for the workflow run. Before running the
check, run **Release** on that commit with GitHub Release and all Docker
publication options disabled so it produces the Linux x86_64 server artifact.
No release or registry changes are made by Upgrade Compatibility.

- **Previous tag** defaults to GitHub's latest published stable release.
  Select an explicit stable tag, for example `v0.8.0`, to repeat a baseline.
- **Candidate run ID** defaults to the newest successful **Release** run for the
  same commit. Supply a run ID to choose a different successful run for that
  commit. A missing, expired, mismatched, or unverifiable artifact fails the job.
- **Require logical restore** makes fresh target table-level export/import a
  required check. It always runs when possible and its result is recorded,
  but it is advisory by default because this gate validates in-place support.
- Artifact **upgrade-compatibility-evidence** contains the machine-readable
  verdict, source and candidate digests, per-check statuses, and server logs.
  Keep the evidence with the candidate as the release SHA changes.

The check starts the released server on loopback, creates SHARED data and two
isolated USER scopes, flushes to Parquet, and adds hot writes. It stops the
server and tests an independent copy on the candidate at the original configured
local-storage path, verifies old
credentials and values, writes new data, and restarts the candidate. It tests
rollback by restoring a separate untouched copy to that same path, plus
table-level export/import into a
fresh candidate with schema and users recreated.

This is a single-node Linux x86_64 synthetic fixture. It does not establish
rolling upgrades, legacy on-disk layouts, remote object storage, full
metadata portability, or recovery of writes made after upgrading when
rolling back. A successful rehearsal is not a reason to skip real backups.

GitHub requires the workflow definition on the default branch for the manual
Actions dispatch entry point. Until then, the same Python script can be run
locally against downloaded and independently verified release binaries:

```bash
python3 scripts/check-upgrade-compatibility.py \
  --local-baseline-server /path/to/previous-server \
  --local-baseline-config /path/to/previous-server.toml \
  --local-candidate-server /path/to/candidate-server \
  --previous-tag v0.8.0
```
