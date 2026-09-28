"""Local mock gates for the isolated C4 restore-target provisioner."""

import copy
import unittest
from pathlib import Path
from unittest.mock import patch

from p0c4_restore_target import (AdmissionError, admit_fresh, compose_document,
                                 identity_for, verify_created, quarantine)


ID = "550e8400-e29b-41d4-a716-446655440000"
SUBNET = "10.251.219.0/24"


def empty_snapshot():
    return {"containers": [], "networks": [], "volumes": [],
            "routes": ["10.0.0.0/8"], "daemon_id": "daemon-1"}


def created(identity):
    project = identity["project"]
    network = identity["network"]
    volume = identity["volume"]
    return {
        "containers": [{"Id": "sha256:pg-only", "Name": "/" + project + "-pg-1",
                        "Config": {"Image": identity["image"], "Labels": {
                            "com.docker.compose.project": project,
                            "com.docker.compose.service": "pg"}},
                        "State": {"Running": True},
                        "HostConfig": {"NetworkMode": network, "PortBindings": {}},
                        "NetworkSettings": {"Ports": {}, "Networks": {network: {"NetworkID": "net-id"}}},
                        "Mounts": [{"Type": "volume", "Name": volume,
                                    "Destination": "/var/lib/postgresql"}]}],
        "networks": [{"Id": "net-id", "Name": network, "Internal": True,
                      "Labels": {"com.docker.compose.project": project},
                      "IPAM": {"Config": [{"Subnet": SUBNET}]} }],
        "volumes": [{"Name": volume, "Mountpoint": "/var/lib/docker/volumes/new/_data",
                     "Labels": {"com.docker.compose.project": project}}],
        "daemon_id": "daemon-1",
    }


class RestoreTargetGates(unittest.TestCase):
    def test_admission_requires_uuid_absence_and_unused_explicit_subnet(self):
        identity = identity_for(ID)
        admit_fresh(identity, SUBNET, empty_snapshot())
        for field, object_name in (("containers", identity["project"]),
                                   ("networks", identity["network"]),
                                   ("volumes", identity["volume"])):
            with self.subTest(field=field):
                snapshot = empty_snapshot()
                snapshot[field] = [{"Name": object_name, "Labels": {}}]
                with self.assertRaises(AdmissionError):
                    admit_fresh(identity, SUBNET, snapshot)
        for bad in ("10.0.1.0/24", "127.0.0.0/24", "0.0.0.0/0", "not-a-subnet"):
            with self.subTest(subnet=bad), self.assertRaises(AdmissionError):
                admit_fresh(identity, bad, empty_snapshot())

    def test_compose_has_only_pinned_pg18_and_private_explicit_network(self):
        identity = identity_for(ID)
        doc = compose_document(identity, SUBNET, Path("/trusted/new"),
                               Path("/reviewed/initdb.sh"))
        self.assertEqual(list(doc["services"]), ["pg"])
        pg = doc["services"]["pg"]
        self.assertEqual(pg["image"], identity["image"])
        self.assertNotIn("ports", pg)
        self.assertEqual(pg["environment"]["POSTGRES_DB"], "postgres")
        self.assertEqual(pg["environment"]["C4_TARGET_DATABASE"], identity["database"])
        self.assertEqual(doc["networks"]["test"]["ipam"]["config"][0]["subnet"], SUBNET)
        self.assertTrue(doc["networks"]["test"]["internal"])
        self.assertEqual(pg["volumes"][0], identity["volume"] + ":/var/lib/postgresql")
        self.assertNotIn("runtime", str(doc))
        self.assertNotIn("worker", str(doc))

    def test_created_identity_requires_exact_labels_ids_mount_and_no_ports(self):
        identity = identity_for(ID)
        expected = created(identity)
        verify_created(identity, SUBNET, empty_snapshot(), expected)
        mutations = [
            lambda s: s["containers"][0]["Mounts"][0].update(Name="old-volume"),
            lambda s: s["containers"][0]["NetworkSettings"]["Networks"].update(external={}),
            lambda s: s["containers"][0]["NetworkSettings"]["Ports"].update({"5432/tcp": [{"HostPort": "15432"}]}),
            lambda s: s["containers"][0]["Config"]["Labels"].update({"com.docker.compose.service": "worker"}),
            lambda s: s["networks"][0].update(Id=""),
            lambda s: s["volumes"][0]["Labels"].update({"com.docker.compose.project": "other"}),
            lambda s: s.update(daemon_id="different"),
            lambda s: s["containers"].append(copy.deepcopy(s["containers"][0])),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                live = copy.deepcopy(expected)
                mutate(live)
                with self.assertRaises(AdmissionError):
                    verify_created(identity, SUBNET, empty_snapshot(), live)

    def test_failure_quarantine_stops_only_exact_labeled_new_container_id(self):
        identity = identity_for(ID)
        live = created(identity)
        live["containers"].append({"Id": "sha256:foreign", "Config": {"Labels": {
            "com.docker.compose.project": "other", "com.docker.compose.service": "pg"}}})
        commands = []
        quarantine(identity, live, lambda *args: commands.append(args))
        self.assertEqual(commands, [("stop", "--time", "1", "sha256:pg-only")])


if __name__ == "__main__":
    unittest.main()
