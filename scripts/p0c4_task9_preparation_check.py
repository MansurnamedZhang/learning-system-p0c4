"""Offline source/specification consistency, never operational Task9 acceptance.

Reads only this checkout's fixed public source paths and preparation matrix.
No network, subprocess, environment configuration, installation, key access or
file writes. Recorded test counts are declarations, not re-execution evidence.
This is a local checkout helper, not a hostile-filesystem admission authority.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys


MATRIX = "docs/task9/b1-preparation-test-matrix.json"
ATTEMPT = "crates/learning-backup/src/destination/attempt.rs"
SOURCES = (
    ATTEMPT,
    "crates/learning-backup/src/destination.rs",
    "crates/learning-backup/src/protection.rs",
    "crates/learning-backup/src/registry.rs",
    "crates/learning-backup/src/complete.rs",
    "crates/learning-backup/src/bin/knowweave-c4-destination.rs",
    "crates/learning-backup/src/full_restore/data/rows.rs",
)
STEPS = ("Admit", "Prepare", "Stage", "BeforeRename", "Rename", "ParentSync",
         "FinalReadback", "TerminalWrite", "TerminalSync", "TerminalReadback", "FinalRecheck")
OLD_STEPS = {"BeforeRename", "ParentSync", "FinalReadback", "TerminalSync", "TerminalReadback", "FinalRecheck"}
PRE_ACTION = {
    "Admit": "task9_admit_prewrite_failure_keeps_disk_identity_unconsumed",
    "Prepare": "task9_prepare_failure_preserves_admission_and_rejects_fresh_attempt",
    "Stage": "task9_stage_failure_preserves_admission_and_rejects_fresh_attempt",
    "Rename": "task9_rename_failure_retains_stage_and_rejects_fresh_attempt",
    "TerminalWrite": "task9_terminal_write_preaction_failure_retains_final_without_authority",
}
OUTCOME_TESTS = {
    "task9_outcome_error_revokes_authority_and_retains_committed_residue",
    "task9_outcome_revocation_wins_before_success_registration",
    "task9_outcome_unwind_revokes_authority_and_releases_lock",
    "task9_outcome_wrong_backup_revokes_authority_without_rewriting_residue",
    "task9_self_consistent_identity_substitutions_never_run_final_use",
}
VARIANTS = ("backup_id", "attempt_id", "destination_dev", "destination_ino",
            "evidence_dev", "evidence_ino", "manifest_sha256", "proof_sha256")
GROUPS = (("SEAM", 11, "existing_evidence_seams"),
          ("BOUNDARY", 6, "operation_boundaries"),
          ("BIND", 8, "cross_boundary_binding_specifications"),
          ("OUTCOME", 8, "cancellation_and_results"),
          ("LIVE", 4, "real_independent_positive_chains"))
CAP = 128 * 1024
TOP_FIELDS = {
    "artifact_kind", "schema", "date", "status", "business_source_commit", "public_base_commit",
    "independent_target", "source_bindings", "existing_destination_steps", "cases",
    "operational_cases_executed", "task9_accepted", "product_consumes_this_file", "permissions_changed",
    "source_overlay", "task9_cross_domain_simulations_executed", "new_task9_acceptance_credit",
    "local_negative_units", "local_outcome_and_binding_units",
}
CASE_FIELDS = {"id", "group", "entry_hook", "stimulus", "expected_obligation",
               "existing_coverage_reference", "requires_real_independent_target", "execution_status"}
OUTCOME_FIELDS = {"status", "platform", "fixture", "result_sha256", "source_sha256",
                  "selected_module_unique_passed", "existing_unique_rerun", "new_unique_passed",
                  "total_task9_prefixed_unique_tests", "tests", "identity_variants_in_one_test",
                  "extra_integration_entries_added", "Task9_integration_cases_executed",
                  "Linux_PG_or_real_FS_credit", "Task9_acceptance_credit", "historical_case_coverage_labels"}
HISTORICAL_FIELDS = {"status", "unique_passed", "platform", "fixture", "unit_result_sha256",
                     "quality_result_sha256", "Linux_PG_or_real_FS_credit", "Task9_acceptance_credit", "receipt_scope"}
BIND_FIELDS = ("backup_id", "attempt_id", "manifest_sha256", "proof_sha256", "source_control_sha256",
               "source_and_build_identity", "registry_generation_and_enrolled_root_identity", "original_operation_ownership")


class Rejected(ValueError):
    pass


def require(condition, code):
    if not condition:
        raise Rejected(code)


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "DUPLICATE_JSON_KEY")
        result[key] = value
    return result


def integer(value, expected):
    return type(value) is int and value == expected


def digest(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def description(value):
    return isinstance(value, str) and bool(value.strip()) and len(value) <= 4096


def integer_literal(text):
    require(len(text.lstrip("-")) <= 20, "INTEGER_LITERAL_TOO_LONG")
    return int(text)


def no_float(_):
    raise Rejected("NON_INTEGER_JSON_NUMBER")


def read_fixed(root, relative):
    require(relative in (MATRIX, *SOURCES), "PATH_NOT_ALLOWLISTED")
    require(root.is_absolute(), "CHECKOUT_ROOT")
    current = root
    for part in (None, *Path(relative).parts):
        if part is not None:
            current = current / part
        item = current.lstat()
        require(not stat.S_ISLNK(item.st_mode) and not getattr(item, "st_file_attributes", 0) & 0x400,
                "LINK_OR_REPARSE_POINT")
        if current != root / relative:
            require(stat.S_ISDIR(item.st_mode), "NON_DIRECTORY_COMPONENT")
    require(stat.S_ISREG(item.st_mode) and 0 < item.st_size <= CAP, "FILE_KIND_OR_SIZE")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
    fd = os.open(current, flags)
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        require((opened.st_dev, opened.st_ino) == (item.st_dev, item.st_ino), "FILE_IDENTITY_CHANGED")
        require(stat.S_ISREG(opened.st_mode) and 0 < opened.st_size <= CAP, "FILE_KIND_OR_SIZE")
        data = stream.read(CAP + 1)
        after = os.fstat(stream.fileno())
    require((opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns) ==
            (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns), "FILE_CHANGED_DURING_READ")
    require(len(data) == opened.st_size <= CAP, "FILE_SIZE_CHANGED")
    return data


def validate(matrix_bytes, sources):
    require(type(matrix_bytes) is bytes and 0 < len(matrix_bytes) <= CAP, "MATRIX_SIZE")
    try:
        matrix = json.loads(matrix_bytes, object_pairs_hook=unique_pairs, parse_int=integer_literal, parse_float=no_float,
                            parse_constant=lambda _: (_ for _ in ()).throw(Rejected("NONFINITE_JSON")))
    except Rejected:
        raise
    except (UnicodeError, ValueError, RecursionError):
        raise Rejected("JSON_SYNTAX") from None
    require(type(matrix) is dict and integer(matrix.get("schema"), 1), "MATRIX_SCHEMA")
    require(set(matrix) == TOP_FIELDS, "MATRIX_FIELDS")
    require(matrix.get("artifact_kind") == "task9_preparation_test_specification_not_authority" and
            matrix.get("status") == "DRAFT_FOR_DESIGN_REVIEW_NOT_IMPLEMENTATION_OR_ACCEPTANCE", "PREPARATION_ONLY")
    require(isinstance(matrix.get("date"), str) and re.fullmatch(r"\d{4}-\d{2}-\d{2}", matrix["date"]) and
            description(matrix.get("source_overlay")), "METADATA_DESCRIPTION")
    for field in ("business_source_commit", "public_base_commit"):
        require(isinstance(matrix.get(field), str) and re.fullmatch(r"[0-9a-f]{40}", matrix[field]), "COMMIT_FORMAT")
    for field in ("task9_accepted", "product_consumes_this_file", "permissions_changed"):
        require(matrix.get(field) is False, "FORBIDDEN_ACCEPTANCE_CLAIM")
    for field in ("operational_cases_executed", "task9_cross_domain_simulations_executed", "new_task9_acceptance_credit"):
        require(integer(matrix.get(field), 0), "FORBIDDEN_EXECUTION_CREDIT")
    target = matrix.get("independent_target")
    require(type(target) is dict and set(target) == {"available", "basis", "statement"} and
            target.get("available") is False and target.get("basis") == "USER_STATEMENT" and description(target.get("statement")),
            "TARGET_PREPARATION_SCOPE")
    require(type(sources) is dict and set(sources) == set(SOURCES) and
            all(type(b) is bytes and 0 < len(b) <= CAP for b in sources.values()),
            "FIXED_SOURCE_SET")
    bindings = matrix.get("source_bindings")
    require(type(bindings) is list and len(bindings) == 7, "SOURCE_BINDINGS")
    seen = set()
    for row in bindings:
        require(type(row) is dict and row.get("path") in SOURCES and row["path"] not in seen, "SOURCE_PATH_OR_DUPLICATE")
        extra = {"baseline_source_sha256", "change_scope"} if row["path"] in (ATTEMPT, SOURCES[-1]) else set()
        require(set(row) == {"path", "bytes", "sha256"} | extra, "SOURCE_BINDING_FIELDS")
        if extra:
            require(digest(row.get("baseline_source_sha256")) and description(row.get("change_scope")), "SOURCE_BASELINE_METADATA")
        seen.add(row["path"])
        data = sources[row["path"]]
        require(integer(row.get("bytes"), len(data)) and digest(row.get("sha256")) and
                hashlib.sha256(data).hexdigest() == row["sha256"], "SOURCE_DIGEST_OR_SIZE")
    source = sources[ATTEMPT].decode("utf-8")
    actual = re.search(r"const STEPS: \[Step; 11\] = \[(.*?)\];", source, re.S)
    require(actual is not None and tuple(re.findall(r"Step::(\w+)", actual[1])) == STEPS and
            matrix.get("existing_destination_steps") == list(STEPS), "STEP_ORDER")
    loop = re.search(r"fn every_seam_retains_residue_blocks_same_id_and_never_reconstructs_live_authority\(\).*?for fail in \[(.*?)\]", source, re.S)
    require(loop is not None and re.findall(r"Step::(\w+)", loop[1]) == [s for s in STEPS if s in OLD_STEPS], "SIX_SEAM_REFERENCE")
    cases = matrix.get("cases")
    require(type(cases) is list and len(cases) == 37, "CASE_COUNT")
    expected = {f"B1-{prefix}-{i:02}": group for prefix, count, group in GROUPS for i in range(1, count + 1)}
    seen = set()
    for case in cases:
        require(type(case) is dict and isinstance(case.get("id"), str) and case["id"] in expected and
                case["id"] not in seen, "CASE_ID_OR_DUPLICATE")
        seen.add(case["id"])
        require(case.get("group") == expected[case["id"]] and
                case.get("execution_status") == "NOT_EXECUTED_AS_TASK9_INTEGRATION_CASE", "CASE_GROUP_OR_EXECUTION")
        extra = {"existing_seam"} if case["id"].startswith("B1-SEAM-") else (
            {"binding_field"} if case["id"].startswith("B1-BIND-") else set())
        require(set(case) == CASE_FIELDS | extra, "CASE_FIELDS")
        require(all(description(case.get(k)) for k in ("entry_hook", "stimulus", "expected_obligation")), "CASE_DESCRIPTION")
        if case["id"].startswith("B1-BIND-"):
            require(case.get("binding_field") == BIND_FIELDS[int(case["id"][-2:]) - 1], "BINDING_FIELD")
        real = case["id"].startswith("B1-LIVE-") or case["id"] in ("B1-OUTCOME-07", "B1-OUTCOME-08")
        require(case.get("requires_real_independent_target") is real, "REAL_TARGET_REQUIREMENT")
        if case["id"].startswith("B1-SEAM-"):
            step = STEPS[int(case["id"][-2:]) - 1]
            ref = case.get("existing_coverage_reference")
            function = ("every_seam_retains_residue_blocks_same_id_and_never_reconstructs_live_authority"
                        if step in OLD_STEPS else PRE_ACTION[step])
            require(case.get("existing_seam") == step and type(ref) is dict and ref.get("path") == ATTEMPT and
                    ref.get("function") == function, "SEAM_REFERENCE")
            keys = {"path", "function", "scope"}
            if step in OLD_STEPS:
                require(set(ref) == keys and ref.get("scope") == "EXACT_SIX_SEAM_SOURCE_REFERENCE_NOT_RERUN", "SEAM_PROVENANCE")
            else:
                require(set(ref) == keys | {"source", "fault_layer", "unit_observation"} and
                        ref.get("scope") == "NEW_LOCAL_WINDOWS_STORE_NEGATIVE_UNIT_NOT_TASK9_EXECUTION" and
                        ref.get("source") == "current candidate cfg(test) overlay, not Source679 ZIP" and
                        ref.get("fault_layer") == "Store::step pre-action failure; not real filesystem" and
                        ref.get("unit_observation") == "PASSED_WINDOWS_LOCAL_UNIT", "SEAM_PROVENANCE")
            require(re.search(r"\bfn\s+" + re.escape(function) + r"\s*\(", source) is not None, "MISSING_TEST_FUNCTION")
        else:
            require(case.get("existing_coverage_reference") is None, "UNDECLARED_CASE_REFERENCE")
    units = matrix.get("local_outcome_and_binding_units")
    require(type(units) is dict and set(units) == OUTCOME_FIELDS and units.get("platform") == "Windows" and
            units.get("status") == "WINDOWS_ATTEMPT_MODULE_PASSED_NOT_TASK9_ACCEPTANCE", "LOCAL_UNIT_SCHEMA")
    require(description(units.get("fixture")) and description(units.get("historical_case_coverage_labels")), "LOCAL_UNIT_DESCRIPTION")
    for field, value in (("selected_module_unique_passed", 19), ("existing_unique_rerun", 14),
                         ("new_unique_passed", 5), ("total_task9_prefixed_unique_tests", 10)):
        require(integer(units.get(field), value), "DECLARED_UNIT_COUNTS")
    for field in ("extra_integration_entries_added", "Task9_integration_cases_executed", "Linux_PG_or_real_FS_credit", "Task9_acceptance_credit"):
        require(integer(units.get(field), 0), "FORBIDDEN_UNIT_CREDIT")
    require(units.get("source_sha256") == hashlib.sha256(sources[ATTEMPT]).hexdigest() and
            digest(units.get("result_sha256")), "LOCAL_UNIT_BINDING")
    tests = units.get("tests")
    require(type(tests) is list and len(tests) == 5 and all(type(t) is dict and set(t) == {"path", "function"} and t.get("path") == ATTEMPT and
            isinstance(t.get("function"), str) for t in tests), "LOCAL_TEST_REFERENCES")
    require({t["function"] for t in tests} == OUTCOME_TESTS and units.get("identity_variants_in_one_test") == list(VARIANTS),
            "LOCAL_TEST_OR_VARIANT_SET")
    for function in OUTCOME_TESTS:
        require(re.search(r"#\[test\]\s+fn\s+" + function + r"\s*\(", source) is not None, "MISSING_TEST_FUNCTION")
    historical = matrix.get("local_negative_units")
    require(type(historical) is dict and set(historical) == HISTORICAL_FIELDS and historical.get("platform") == "Windows" and
            historical.get("status") == "FIVE_LOCAL_WINDOWS_NEGATIVE_UNITS_PASSED_NOT_TASK9_ACCEPTANCE" and
            historical.get("receipt_scope") == "HISTORICAL_FIRST_FIVE_WINDOWS_PRE_ACTION_TESTS_CURRENTLY_RERUN_IN_MODULE19" and
            description(historical.get("fixture")) and
            integer(historical.get("unique_passed"), 5) and
            digest(historical.get("unit_result_sha256")) and digest(historical.get("quality_result_sha256")),
            "HISTORICAL_UNIT_SCHEMA")
    for field in ("Linux_PG_or_real_FS_credit", "Task9_acceptance_credit"):
        require(integer(historical.get(field), 0), "FORBIDDEN_HISTORICAL_UNIT_CREDIT")
    return {"status": "TASK9_PREPARATION_STRUCTURE_CHECKED_NOT_ACCEPTANCE", "schema": 1,
            "matrix_sha256": hashlib.sha256(matrix_bytes).hexdigest(), "source_files_checked": 7,
            "specification_entries_checked": 37, "source_text_step_references_checked": 11,
            "declared_windows_module_tests": 19, "declared_new_outcome_tests": 5,
            "test_execution_evidence_verified": False, "tests_executed": False,
            "Task9_integration_cases_executed": 0, "Task9_accepted": False,
            "permissions_changed": False, "product_consumes_report": False}


def main():
    try:
        require(len(sys.argv) == 1, "ARGUMENTS_NOT_SUPPORTED")
        root = Path(__file__).absolute().parent.parent
        matrix = read_fixed(root, MATRIX)
        sources = {name: read_fixed(root, name) for name in SOURCES}
        report = validate(matrix, sources)
    except (Rejected, OSError, UnicodeError) as error:
        code = str(error) if isinstance(error, Rejected) else "LOCAL_INPUT_UNREADABLE"
        print(json.dumps({"status": "PREPARATION_CHECK_REJECTED_NOT_ACCEPTANCE", "reason_code": code,
                          "tests_executed": False, "Task9_accepted": False, "permissions_changed": False}))
        return 1
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
