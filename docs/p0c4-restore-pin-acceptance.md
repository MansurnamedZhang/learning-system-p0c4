# P0-C4 clean target pin candidate: single-host acceptance

This root-only runner creates one new PostgreSQL 18 Compose project for a **new canonical UUIDv4** and an explicit unused RFC1918 subnet. It does not read a CompleteBackup, restore any data, alter ACLs, pin a build, or admit a service. A passing result is only a clean, single-host pin candidate for later independent review. It does not prove an independent failure domain.

Package exactly the reviewed committed source with `python scripts/package_p0c4_task3.py --repository . --output .artifacts/p0c4-pin-candidate-reviewed.zip`. Record its ZIP SHA-256, manifest SHA-256, source commit, and the SHA-256 of `scripts/p0c4_restore_pin_acceptance.py`. Each Linux upload and run needs separate authorization for those exact bytes. The commands below describe a later authorized run; do not substitute a previous birth-acceptance batch, UUID, volume, or subnet.

Place the approved ZIP under `/var/lib/knowweave-c4/incoming/` as root-owned `0400`, and the exact runner under `/var/lib/knowweave-c4/tools/` as root-owned `0500`. The `BASE`, `incoming`, and `tools` directories must be root-owned `0700` with trusted ancestors. The pinned PG18 image must already be local. Choose a subnet that does not overlap any host route or Docker network. In one SSH shell, verify both transferred hashes and run:

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
  --subnet UNUSED_RFC1918_CIDR
```

The runner verifies the whole ZIP, canonical manifest, every tracked member, and its own installed bytes. It extracts source into a new root-private batch and checks that source again after the run. Within a fresh private `control` root, it records target absence under the provisioner's creation lock **before** invoking the birth issuer. It then checks the canonical sealed birth, Docker image/network/container/volume/mount identities, PG18 facts, empty restore roots, and clean ACL through the read-only pin checker. Only after the confirmed immutable PG container ID has been stopped and the volume is still present does it write `PIN_CANDIDATE_SINGLE_HOST_PG18_PASSED_QUARANTINED_NOT_RESTORE` with `birth_sha256` and `inspection_evidence_sha256` to the private result and stdout summary. The test volume remains quarantined; there is no automatic build pin.

The evidence is under `/var/lib/knowweave-c4/pin-acceptance/batches/<UUID>/evidence/`. `attempt.json` is written before birth begins; `result.json` records a redacted stage, hashes, and exact-stop status. A failed or interrupted batch is never replayed. If `stop.confirmed=false`, the runner could not prove the PG container is stopped and an operator must inspect that exact batch before any further action. An abrupt SIGKILL or host crash may leave only `attempt.json`; this is **not** a candidate and may require the same exact-ID quarantine review. Keep the failed evidence and volume. Never use a previous birth-acceptance batch: its ACL was deliberately dirtied.

Local Python tests cover orchestration and failure gates using mocks. They do not establish actual Linux root, Docker, or PostgreSQL 18 behavior; that requires the separately approved isolated host run with the exact reviewed package and runner.
