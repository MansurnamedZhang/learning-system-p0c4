# P0-C4 read-only bound-target probe on a new Linux PG18 target

This is a separate, opt-in single-host gate for the private Rust Docker/PG probe. It creates a **new** canonical UUIDv4 batch through the existing pin-only runner. After the birth issuer and pin inspection, while the isolated PG18 container is still running, it builds and runs exactly one ignored Rust test. The runner then stops that exact container and retains its volume in quarantine. It does not read a CompleteBackup, restore a dump, create a build pin, or admit a service. Never reuse the stopped pin candidate or a birth-acceptance batch.

Prepare a reviewed source ZIP and installed runner as described in [pin candidate acceptance](p0c4-restore-pin-acceptance.md), using the final committed source of this gate. Verify the ZIP, manifest, runner, and commit hashes on the host. The Linux host needs a root-owned canonical Cargo executable and Rust compiler compatible with `rust-version = 1.97`; root's Cargo cache must contain every locked dependency because the probe uses `--offline --locked`. `/usr/bin/docker` must be trusted and point at the local daemon. The pinned PG18 image and an unused nonoverlapping RFC1918 subnet must already be available. No Docker, Cargo, or Rust compiler is installed or changed by this gate.

Use one user-operated root command with an **unused** UUIDv4 and subnet. `CARGO` must be the resolved executable file, not a symlink; each ancestor must be root-owned and not writable by group or others. The runner rejects an unavailable compiler, missing offline dependency, failed Linux compilation, zero executed tests, target mismatch, or Docker/PG drift, and still attempts exact-ID quarantine.

```sh
printf '%s  %s\n' EXACT_RUNNER_SHA256 /var/lib/knowweave-c4/tools/p0c4_restore_pin_acceptance.py | sudo sha256sum --check || exit 1
printf '%s  %s\n' EXACT_ZIP_SHA256 /var/lib/knowweave-c4/incoming/REVIEWED.zip | sudo sha256sum --check || exit 1
sudo /usr/bin/python3 -B /var/lib/knowweave-c4/tools/p0c4_restore_pin_acceptance.py \
  --archive /var/lib/knowweave-c4/incoming/REVIEWED.zip \
  --archive-sha256 EXACT_ZIP_SHA256 \
  --manifest-sha256 EXACT_MANIFEST_SHA256 \
  --source-commit EXACT_40_HEX_COMMIT \
  --runner-sha256 EXACT_RUNNER_SHA256 \
  --batch-id NEW_CANONICAL_UUIDV4 \
  --subnet UNUSED_RFC1918_CIDR \
  --bound-probe-cargo /ABSOLUTE/ROOT_OWNED/RESOLVED/bin/cargo
```

The runner passes only the sealed birth SHA-256 and four nonsecret target values to Cargo: `KNOWWEAVE_C4_PROBE_DESTINATION_ROOT`, `KNOWWEAVE_C4_PROBE_CONTROL_ROOT`, `KNOWWEAVE_C4_PROBE_ASSET_ROOT`, and `KNOWWEAVE_C4_PROBE_EXPECTED_DATABASE`. Cargo builds from the hash-checked extracted source with a separate private build directory. The ignored test requires root, the compile-time birth SHA, existing private roots and locks, the pin-only precreation record, birth/issuance evidence, exact Docker objects and mounts, and matching PostgreSQL system identifier and database OID before and after its read-only query. It never creates a restore lock. Rust test output is used only to check the exact one-test success marker; raw output and credentials are not stored in the result.

Accept this gate only if the calling shell observes exit code 0, the final `result.json` has status `BOUND_TARGET_READ_ONLY_SINGLE_HOST_PG18_PASSED_QUARANTINED_NOT_RESTORE`, its raw SHA-256 matches the printed `result_sha256`, `bound_probe.state` is `BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE` with the same `birth_sha256`, the pin inspection evidence hash matches, `stop.confirmed=true`, `stop.volume_retained=true`, and `result.pending.json` is absent. A failure has status `BOUND_TARGET_READ_ONLY_FAILED_QUARANTINED_NOT_RESTORE_NOT_PIN`; a failed or interrupted batch must never be replayed. If stop is unconfirmed, inspect that exact project before any further operation. This gate is still a single-host observation and does not satisfy the independent-target, restore, or complete C4 acceptance gates.
