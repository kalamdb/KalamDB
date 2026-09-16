# CLI server discovery

The CLI tracks managed server locations in `~/.kalam/server-registry/`.
Each file contains an absolute instance-file path and working folder. Filenames
are SHA-256 hashes of canonical instance paths, so repeated registration does
not duplicate entries. Independent atomic writes avoid a shared read-modify-write
index and preserve concurrent registrations from different folders.

The per-instance `run/instance.json` remains authoritative for URL, paths, and
process identity. Listing verifies PID/executable identity instead of inferring
ownership from an open port. Stopped instances remain listed. Missing or
unreadable records are unavailable; one broken entry does not block others.

Successful managed starts and reuse register the location. Status can import
older instances, and listing imports the current-folder and default-global
instances when present. Registration failures produce a warning without failing
an otherwise successful server start. The index contains no credentials.

This is a registry of CLI-managed instances, not an OS process scanner. A server
launched manually cannot be reliably mapped to a URL and folder on every platform.
An empty folder's status is not configured; it does not probe the default port.

The unified instance inventory joins registry pointers with the existing credential
store. Local URL matches suppress duplicate credential rows and hide authentication
details. Credentials remain in their original secure store. Local folder names and
the global name become lifecycle selectors; collisions receive distinct suffixes,
and ambiguous aliases fail instead of selecting a process. Named local operations
resolve the registered folder before discovering project configuration.

Named diagnostics use saved endpoint-matched credentials and the SDK refresh
callback. Cloud status and logs query existing system.cluster and
system.server_logs views; live local instances use the same path when matching
credentials exist. Offline local state and capture files remain independent of
authentication. The --local-file option forces local file inspection. SQL log
following polls timestamp-ordered pages with occurrence counts at the boundary
to preserve repeated events without replaying the previous snapshot. The initial
tail includes timestamp ties for cursor initialization. This is node-local,
timestamp-based polling, not a durable distributed log cursor.
