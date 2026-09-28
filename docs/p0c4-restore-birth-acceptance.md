# P0-C4 fresh-target birth issuer: isolated single-host acceptance

This is a root-only **test** for the opt-in target birth issuer. It creates a
new PostgreSQL 18 Compose project, checks its birth records independently,
then deliberately grants the runtime role `CREATE` on `public` as a negative
case. The runner stops only the verified test container and **retains the
dirty volume**. Its output is never CompleteBackup, a restore result, a build
pin, or production admission.

Before running on Linux, a reviewer must approve one exact source ZIP,
manifest SHA-256, Git commit and runner SHA-256. Install that exact runner at a
root-owned trusted path with mode `0500`. Put the exact ZIP under
`/var/lib/knowweave-c4/incoming/` with mode `0400`. The
`/var/lib/knowweave-c4` and `incoming` directories must be root-owned
`0700`, with trusted, non-writable ancestors. Docker and its local operator
are trusted. The pinned PostgreSQL image must already be local. Choose a new
canonical UUIDv4 and a previously unused explicit RFC1918 `/24` (or smaller)
that does not overlap a host route or Docker network.

Run the following commands in order in the **same SSH shell**. Each hash
failure exits that shell immediately; do not skip or ignore a nonzero result.
The runner also checks the approved bytes and manifest internally.

```sh
printf '%s  %s\n' EXACT_RUNNER_SHA256 /var/lib/knowweave-c4/tools/p0c4_restore_birth_acceptance.py | sudo sha256sum --check || exit 1
printf '%s  %s\n' EXACT_ZIP_SHA256 /var/lib/knowweave-c4/incoming/REVIEWED.zip | sudo sha256sum --check || exit 1
sudo /var/lib/knowweave-c4/tools/p0c4_restore_birth_acceptance.py \
  --archive /var/lib/knowweave-c4/incoming/REVIEWED.zip \
  --archive-sha256 EXACT_ZIP_SHA256 \
  --manifest-sha256 EXACT_MANIFEST_SHA256 \
  --source-commit EXACT_40_HEX_COMMIT \
  --batch-id NEW_CANONICAL_UUIDV4 \
  --subnet UNUSED_RFC1918_CIDR
```

The runner prints one summary with `status`, `result_sha256`, and a
root-private evidence path. A passing status still ends in
`DIRTY_UNUSABLE_NOT_RESTORE_NOT_PIN`. On failure, inspect only this batch's
root-private `result.json`; `stop.confirmed=false` means isolation could
not be proven. Never rerun a failed batch ID or reuse its volume. Do not send
passwords, secret files, or full connection strings to chat.

Local Python mock tests exercise the protocol and fault paths. They do not
establish real Linux, Docker, or PostgreSQL 18 behavior; that requires this
exact reviewed runner and source archive on an isolated host.
