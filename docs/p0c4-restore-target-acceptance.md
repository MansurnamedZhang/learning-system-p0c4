# P0-C4 fresh restore-target PG18 acceptance candidate

This runner tests only the reviewed fresh restore-target provisioner on an
isolated Linux host. It creates one new Compose project and dedicated PG18
database from a fresh UUIDv4, then grants `CREATE` on that database's `public`
schema to `learning_runtime` as a negative test. A pass means the reviewed
probe returned `REJECT` after that deliberate mutation. The retained volume is
**dirty and quarantined**; it must never be used for a birth attestation,
`CompleteBackup`, restore, or production admission. The provisioner's
`state.json` predates the negative mutation and is only a creation record.

First build the source archive locally from the exact reviewed HEAD. The
packager reads Git blobs, so untracked and working-tree edits are excluded:

```powershell
python scripts/package_p0c4_task3.py --repository . --output .artifacts/p0c4-restore-target-reviewed.zip
```

Record the printed archive SHA-256, manifest SHA-256, source commit and the
SHA-256 of `scripts/p0c4_restore_target_acceptance.py`. The separate host-run
approval must name **both** the exact ZIP SHA-256 and runner SHA-256. Review
those values before any separate transfer to the isolated Linux host. No
transfer or remote execution is part of this repository task.

On the isolated host, put the approved ZIP and this exact runner under the
root-private control root, verify both transferred hashes, and run once with
an explicit unused RFC1918 `/24` (or smaller) subnet. These are example
commands for a separately authorized host run; replace the placeholders with
the reviewed literal values before execution:

```sh
sudo install -d -o root -g root -m 0700 /var/lib/knowweave-c4
sudo install -d -o root -g root -m 0700 /var/lib/knowweave-c4/tools /var/lib/knowweave-c4/incoming /var/lib/knowweave-c4/batches
sudo install -o root -g root -m 0500 p0c4_restore_target_acceptance.py /var/lib/knowweave-c4/tools/p0c4_restore_target_acceptance.py
sudo install -o root -g root -m 0400 p0c4-restore-target-reviewed.zip /var/lib/knowweave-c4/incoming/p0c4-restore-target-reviewed.zip
printf '%s  %s\n' '<exact-approved-runner-sha256>' /var/lib/knowweave-c4/tools/p0c4_restore_target_acceptance.py | sudo sha256sum --check || exit 1
printf '%s  %s\n' '<exact-approved-archive-sha256>' /var/lib/knowweave-c4/incoming/p0c4-restore-target-reviewed.zip | sudo sha256sum --check || exit 1
sudo python3 -B /var/lib/knowweave-c4/tools/p0c4_restore_target_acceptance.py \
  --archive /var/lib/knowweave-c4/incoming/p0c4-restore-target-reviewed.zip \
  --archive-sha256 '<exact-archive-sha256>' \
  --manifest-sha256 '<exact-manifest-sha256>' \
  --source-commit '<exact-commit>' \
  --subnet '<unused-rfc1918-subnet>'
```

The runner verifies every regular tracked file against the canonical manifest
before private extraction and checks its own bytes against that manifest. It
installs the initdb script root-owned at mode `0444` under trusted ancestors,
creates `0700` control and target directories, calls the provisioner, checks
the dedicated database and PG18 version, inspects the immutable Docker object
IDs, labels, volume mount, private network, no published ports, and the exact
read-only initdb and secret binds. It captures only redacted hashes and facts
in root-private `evidence/result.json`, then stops only its verified PG
container ID and retains the volume. If the provisioner fails before returning
an ID, its own quarantine path handles the early stop attempt; the runner
reads that provisioner's private `failure.json` and records a project-only
container snapshot, without trying to stop an unverified container. From the
start of provisioning the target is reported as unusable even if its precise
condition is unknown. A failed batch is never resumed; any subsequent attempt
must use a new invocation, UUID and project after review of the prior failure.
