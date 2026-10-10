# P0-C4 internal bound-target guard Linux gate

This opt-in gate exercises the private `BoundTargetGuard` on a **new** isolated PG18 target. It extends the reviewed pin-only acceptance path with `--bound-guard`, mutually exclusive with `--bound-probe`. The existing probe still does not create a target lock. The guard gate deliberately creates and retains one empty target lock file; it does not create a restore-attempt marker, import a dump, read a CompleteBackup, create a deployment build pin, or admit a service.

Use the source packaging, root-private incoming directory, exact runner/source/manifest hashes, pinned PG18 image, unused UUIDv4 and nonoverlapping subnet requirements in [pin acceptance](p0c4-restore-pin-acceptance.md). Never reuse any prior stopped pin, probe, guard, or birth batch. The completed single-host read-only run and its evidence are recorded in [C4 verification](p0c4-verification.md); this procedure alone is not acceptance evidence.

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
  --bound-guard
```

Before target birth, the runner reuses the exact offline builder image `sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b`, network-disabled build, immutable source mount, locked dependencies, batch-scoped builder cleanup, and host-loader preflight from [the probe gate](p0c4-bound-target-probe.md). It requires the exact guard test in the host `--list` output. After birth and durable pin inspection it rebuilds with the sealed birth digest and executes only `restore_preflight::target_binding::tests::live_read_only_bound_target_guard --exact --ignored --nocapture`. Only four nonsecret target values are supplied, using the existing `KNOWWEAVE_C4_PROBE_*` environment keys. The compile-time birth digest is a test-binary identity check, not a deployment build-pin artifact.

The Linux test requires an untouched target with only its birth file in control and empty destination/assets. It holds the global creation flock on an independently opened descriptor and proves `acquire_for_restore` fails without creating a target lock or changing files. After dropping that descriptor, it calls `acquire_for_restore` successfully. It checks the guard owns the newly created target lock inode, root ownership, single link, mode 0600 and zero length. Separate descriptors must receive `WouldBlock` for both global and target flocks while the guard lives. It rechecks the guard's original Docker/PG identities, checks that user relations, large objects and extra user schemas remain absent using catalog SELECTs over the exact container ID/local socket path, and compares target control/destination/assets file snapshots. The only permitted file change is the empty target lock; a restore-attempt marker or data file fails the gate. After `drop(guard)`, both independent descriptors must acquire their locks, and the file snapshot and absent attempt marker are checked again. Test descriptors are dropped before the harness reports the test passed.

Accept only exit code 0 **and** all of:

- Final `result.json.status` equals `BOUND_TARGET_GUARD_READ_ONLY_SINGLE_HOST_PG18_PASSED_QUARANTINED_NOT_RESTORE` and its raw SHA-256 equals printed `result_sha256`; no pending result remains.
- `bound_guard.state` equals `BOUND_TARGET_GUARD_READ_ONLY_PG18_PASSED_NOT_RESTORE`, its birth digest equals the sealed candidate digest, and its binary digest and pinned builder ID are recorded. The runner checks one exact guard marker, the selected test name, `running 1 test`, one successful one-test summary, exit code 0 and unchanged binary hash. Probe, zero-test, multi-test, duplicate marker, or decorated marker output cannot pass.
- `guard_toolchain_preflight.host_test_listing_confirmed=true`; reviewed source hashes before/after match; source ZIP/manifest/runner/commit identities and durable pin-inspection evidence hashes remain verified.
- The exact verified PG container was stopped: `stop.confirmed=true` and `stop.volume_retained=true`. Its volume and the new target lock remain quarantined; `target_reuse_permitted=false`.

Failures use `BOUND_TARGET_GUARD_READ_ONLY_FAILED_QUARANTINED_NOT_RESTORE_NOT_PIN`. A preflight failure creates no PG target; a later failure attempts the same exact-ID stop and retained-volume quarantine. If stop is unconfirmed, inspect only that exact project before further operations. Never replay a failed/interrupted batch.

Limits: this is a single-host lock/identity/no-persistent-application-data observation, not tracing every filesystem write by PostgreSQL (which updates its own runtime files) or excluding transient writes by a hostile external process. It does not prove a SQLx pool or host `pg_restore` connection reaches that Docker endpoint, and does not satisfy restore, independent-target, or complete C4 acceptance. No remote upload or live Linux execution was performed while implementing this gate. The local Windows checks do not compile or exercise Linux-only code; the pinned offline preflight remains mandatory before any live target is issued.

Local implementation validation: Python suite 127 passed; Rust `learning-backup --lib` 22 passed; workspace strict Clippy and formatting passed on Windows. The full workspace Rust suite was attempted and stopped at `admin_collects_linked_and_unlinked_ready_rows_and_runtime_is_rejected` because its dedicated empty backup-test database environment was absent (`NotPresent`); no database credentials were requested and that prerequisite was not bypassed.
