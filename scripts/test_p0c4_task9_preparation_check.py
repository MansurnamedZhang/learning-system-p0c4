"""Local structural-checker regressions. These never execute Task9 cases."""
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import stat
import tempfile
import types
import unittest
from unittest.mock import patch

import p0c4_task9_preparation_check as check


class PreparationCheckTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        root = Path(__file__).absolute().parent.parent
        cls.raw = check.read_fixed(root, check.MATRIX)
        cls.matrix = json.loads(cls.raw)
        cls.sources = {name: check.read_fixed(root, name) for name in check.SOURCES}

    def mutate(self, edit, code):
        matrix = copy.deepcopy(self.matrix)
        edit(matrix)
        with self.assertRaisesRegex(check.Rejected, "^" + code + "$"):
            check.validate(json.dumps(matrix).encode(), self.sources)

    def test_current_structure_does_not_certify_recorded_execution(self):
        report = check.validate(self.raw, self.sources)
        self.assertEqual(report["specification_entries_checked"], 37)
        self.assertEqual(report["declared_windows_module_tests"], 19)
        self.assertFalse(report["test_execution_evidence_verified"])
        self.assertFalse(report["tests_executed"])
        self.assertFalse(report["Task9_accepted"])
        self.assertEqual(report["Task9_integration_cases_executed"], 0)

    def test_acceptance_claim_is_rejected(self):
        self.mutate(lambda m: m.update(task9_accepted=True), "FORBIDDEN_ACCEPTANCE_CLAIM")

    def test_product_consumer_claim_is_rejected(self):
        self.mutate(lambda m: m.update(product_consumes_this_file=True), "FORBIDDEN_ACCEPTANCE_CLAIM")

    def test_execution_credit_is_rejected(self):
        self.mutate(lambda m: m.update(operational_cases_executed=37), "FORBIDDEN_EXECUTION_CREDIT")

    def test_boolean_zero_is_not_an_execution_count(self):
        self.mutate(lambda m: m.update(operational_cases_executed=False), "FORBIDDEN_EXECUTION_CREDIT")

    def test_unknown_top_level_permission_is_rejected(self):
        self.mutate(lambda m: m.update(allow_signing=True), "MATRIX_FIELDS")

    def test_duplicate_json_key_is_rejected(self):
        raw = b'{"schema":1,' + self.raw.lstrip()[1:]
        with self.assertRaisesRegex(check.Rejected, "^DUPLICATE_JSON_KEY$"):
            check.validate(raw, self.sources)

    def test_nonfinite_json_is_rejected(self):
        raw = self.raw.replace(b'"schema": 1', b'"schema": NaN', 1)
        with self.assertRaisesRegex(check.Rejected, "^NONFINITE_JSON$"):
            check.validate(raw, self.sources)

    def test_exponent_overflow_is_rejected_before_becoming_infinity(self):
        raw = self.raw.replace(b'"date": "2026-10-26"', b'"date": 1e400', 1)
        with self.assertRaisesRegex(check.Rejected, "^NON_INTEGER_JSON_NUMBER$"):
            check.validate(raw, self.sources)

    def test_float_descriptor_is_rejected(self):
        raw = self.raw.replace(b'"date": "2026-10-26"', b'"date": 0.5', 1)
        with self.assertRaisesRegex(check.Rejected, "^NON_INTEGER_JSON_NUMBER$"):
            check.validate(raw, self.sources)

    def test_long_integer_is_a_structured_rejection(self):
        raw = self.raw.replace(b'"schema": 1', b'"schema": ' + b'9' * 5000, 1)
        with self.assertRaisesRegex(check.Rejected, "^INTEGER_LITERAL_TOO_LONG$"):
            check.validate(raw, self.sources)

    def test_empty_description_is_rejected(self):
        self.mutate(lambda m: m["cases"][0].update(entry_hook=" "), "CASE_DESCRIPTION")

    def test_numeric_description_is_rejected(self):
        self.mutate(lambda m: m["cases"][0].update(expected_obligation=5), "CASE_DESCRIPTION")

    def test_nested_target_permission_is_rejected(self):
        self.mutate(lambda m: m["independent_target"].update(allow_signing=True), "TARGET_PREPARATION_SCOPE")

    def test_nested_source_permission_is_rejected(self):
        self.mutate(lambda m: m["source_bindings"][0].update(Task9_accepted=True), "SOURCE_BINDING_FIELDS")

    def test_nested_unit_permission_is_rejected(self):
        self.mutate(lambda m: m["local_outcome_and_binding_units"].update(allow_signing=True), "LOCAL_UNIT_SCHEMA")

    def test_nested_historical_permission_is_rejected(self):
        self.mutate(lambda m: m["local_negative_units"].update(allow_signing=True), "HISTORICAL_UNIT_SCHEMA")

    def test_forged_fresh_domain_reference_is_rejected(self):
        self.mutate(lambda m: m["cases"][3]["existing_coverage_reference"].update(scope="FRESH_DOMAIN_PASSED"), "SEAM_PROVENANCE")

    def test_new_reference_cannot_claim_physical_io(self):
        self.mutate(lambda m: m["cases"][0]["existing_coverage_reference"].update(fault_layer="REAL_FS"), "SEAM_PROVENANCE")

    def test_unreviewed_reference_on_other_case_is_rejected(self):
        self.mutate(lambda m: m["cases"][-1].update(existing_coverage_reference={"allow_recovery": True}), "UNDECLARED_CASE_REFERENCE")

    def test_oversize_matrix_is_rejected_before_parsing(self):
        with self.assertRaisesRegex(check.Rejected, "^MATRIX_SIZE$"):
            check.validate(b" " * (check.CAP + 1), self.sources)

    def test_source_drift_is_rejected(self):
        sources = dict(self.sources)
        sources[check.ATTEMPT] += b"\n"
        with self.assertRaisesRegex(check.Rejected, "^SOURCE_DIGEST_OR_SIZE$"):
            check.validate(self.raw, sources)

    def test_source_path_traversal_is_rejected(self):
        self.mutate(lambda m: m["source_bindings"][0].update(path="../../private-fixture"), "SOURCE_PATH_OR_DUPLICATE")

    def test_duplicate_source_binding_is_rejected(self):
        self.mutate(lambda m: m["source_bindings"].__setitem__(1, copy.deepcopy(m["source_bindings"][0])),
                    "SOURCE_PATH_OR_DUPLICATE")

    def test_duplicate_case_cannot_hide_missing_case(self):
        self.mutate(lambda m: m["cases"].__setitem__(1, copy.deepcopy(m["cases"][0])), "CASE_ID_OR_DUPLICATE")

    def test_executed_case_label_is_rejected(self):
        self.mutate(lambda m: m["cases"][0].update(execution_status="PASSED"), "CASE_GROUP_OR_EXECUTION")

    def test_real_chain_cannot_be_downgraded_to_local(self):
        self.mutate(lambda m: m["cases"][-1].update(requires_real_independent_target=False), "REAL_TARGET_REQUIREMENT")

    def test_extra_case_permission_is_rejected(self):
        self.mutate(lambda m: m["cases"][0].update(allow_recovery=True), "CASE_FIELDS")

    def test_wrong_seam_function_is_rejected(self):
        self.mutate(lambda m: m["cases"][0]["existing_coverage_reference"].update(function="unrelated_function"), "SEAM_REFERENCE")

    def test_eight_variants_cannot_be_counted_as_eight_new_bodies(self):
        self.mutate(lambda m: m["local_outcome_and_binding_units"].update(new_unique_passed=8), "DECLARED_UNIT_COUNTS")

    def test_local_units_cannot_award_linux_credit(self):
        self.mutate(lambda m: m["local_outcome_and_binding_units"].update(Linux_PG_or_real_FS_credit=1), "FORBIDDEN_UNIT_CREDIT")

    def test_historical_units_cannot_award_acceptance_credit(self):
        self.mutate(lambda m: m["local_negative_units"].update(Task9_acceptance_credit=1), "FORBIDDEN_HISTORICAL_UNIT_CREDIT")

    def test_unit_receipt_must_bind_current_source(self):
        self.mutate(lambda m: m["local_outcome_and_binding_units"].update(source_sha256="0" * 64), "LOCAL_UNIT_BINDING")

    def test_missing_test_function_rejected_even_after_digest_refresh(self):
        sources = dict(self.sources)
        function = next(iter(check.OUTCOME_TESTS))
        sources[check.ATTEMPT] = sources[check.ATTEMPT].replace(
            ("fn " + function + "(").encode(), b"fn removed_test_function(", 1)
        matrix = copy.deepcopy(self.matrix)
        sha = hashlib.sha256(sources[check.ATTEMPT]).hexdigest()
        matrix["source_bindings"][0].update(bytes=len(sources[check.ATTEMPT]), sha256=sha)
        matrix["local_outcome_and_binding_units"]["source_sha256"] = sha
        with self.assertRaisesRegex(check.Rejected, "^MISSING_TEST_FUNCTION$"):
            check.validate(json.dumps(matrix).encode(), sources)

    def test_fixed_reader_rejects_unlisted_path_before_access(self):
        with patch.object(Path, "lstat", side_effect=AssertionError("metadata accessed")):
            with self.assertRaisesRegex(check.Rejected, "^PATH_NOT_ALLOWLISTED$"):
                check.read_fixed(Path("D:/public-fixture"), "../private-fixture")

    def test_fixed_reader_rejects_link_before_open(self):
        fake = types.SimpleNamespace(st_mode=stat.S_IFLNK | 0o777, st_file_attributes=0)
        with patch.object(Path, "lstat", return_value=fake), patch.object(check.os, "open", side_effect=AssertionError("opened")):
            with self.assertRaisesRegex(check.Rejected, "^LINK_OR_REPARSE_POINT$"):
                check.read_fixed(Path.cwd(), check.MATRIX)

    def test_fixed_reader_rejects_large_file_without_open(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = root / check.MATRIX
            path.parent.mkdir(parents=True)
            path.write_bytes(b"x" * (check.CAP + 1))
            with patch.object(check.os, "open", side_effect=AssertionError("opened")):
                with self.assertRaisesRegex(check.Rejected, "^FILE_KIND_OR_SIZE$"):
                    check.read_fixed(root, check.MATRIX)

    def test_cli_rejects_arguments_without_reading_files(self):
        output = io.StringIO()
        with patch.object(check.sys, "argv", ["checker", "--allow-signing"]), \
                patch.object(check, "read_fixed", side_effect=AssertionError("read")), \
                contextlib.redirect_stdout(output):
            self.assertEqual(check.main(), 1)
        report = json.loads(output.getvalue())
        self.assertEqual(report["reason_code"], "ARGUMENTS_NOT_SUPPORTED")
        self.assertFalse(report["Task9_accepted"])


if __name__ == "__main__":
    unittest.main()
