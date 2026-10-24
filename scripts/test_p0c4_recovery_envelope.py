"""Task6 foundation results cannot inherit Task5 or full-closure credit."""
import unittest
from itertools import combinations
from scripts import p0c4_completion_acceptance as runner
from scripts.p0c4_completion import full_import


ORIGINAL_FULL_CASES = {
    'full-import': 'full_restore::live_tests::full_import_preserves_one_writer_transaction_and_exact_endpoint',
    'full-precommit-eof': 'full_restore::live_tests::full_import_eof_before_commit_leaves_zero_objects',
    'full-cancel': 'full_restore::live_tests::full_import_cancel_retains_guards_until_isolation',
    'full-commit-unknown': 'full_restore::live_tests::full_import_commit_unknown_is_unusable_and_no_replay',
    'full-wrong-endpoint': 'full_restore::live_tests::full_import_wrong_endpoint_writes_nothing',
    'full-bad-input': 'full_restore::live_tests::full_import_bad_dump_dirty_target_or_bad_role_writes_nothing',
    'full-bad-role': 'full_restore::live_tests::full_import_bad_role_writes_nothing',
    'full-dirty-target': 'full_restore::live_tests::full_import_dirty_target_writes_nothing',
    'full-asset-corrupt': 'full_restore::live_tests::full_assets_corruption_blocks_recovery_receipt',
}
ATTENTION_PREFIX = ('restore_preflight::target_binding::controlled_import::full::'
                    'attention_tests::')
ATTENTION_CASES = {
    'attention-full-chain': ATTENTION_PREFIX + 'attention_full_chain_preserves_authoritative_inventory_and_originals',
    'attention-missing-asset': ATTENTION_PREFIX + 'attention_missing_asset',
    'attention-corrupt-dump': ATTENTION_PREFIX + 'attention_corrupt_dump',
    'attention-interrupted-transfer': ATTENTION_PREFIX + 'attention_interrupted_transfer',
    'attention-unknown-effect': ATTENTION_PREFIX + 'attention_unknown_effect',
    'attention-root-replacement': ATTENTION_PREFIX + 'attention_root_replacement',
}


class RecoveryEnvelopeTests(unittest.TestCase):
    def test_all_five_recovery_cases_have_an_explicit_nonclosure_result(self):
        self.assertEqual(len(full_import.RECOVERY_CASE_TESTS), 5)
        for case in full_import.RECOVERY_CASE_TESTS:
            self.assertEqual(runner.full_case_pass_status(case),
                'RECOVERY_FOUNDATION_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_TASK6_COMPLETE')
            self.assertNotEqual(runner.full_case_pass_status(case), runner.FULL_PASS)

    def test_existing_nine_full_cases_keep_their_original_status(self):
        cases = (set(full_import.CASE_TESTS) - set(full_import.RECOVERY_CASE_TESTS)
                 - set(full_import.SNAPSHOT_CASE_TESTS) - set(full_import.ATTENTION_CASE_TESTS))
        self.assertEqual(cases, set(ORIGINAL_FULL_CASES))
        self.assertEqual(len(cases), 9)
        for case in cases:
            with self.subTest(case=case):
                self.assertEqual(full_import.CASE_TESTS[case], ORIGINAL_FULL_CASES[case])
                self.assertEqual(runner.full_case_pass_status(case), runner.FULL_PASS)

    def test_six_attention_cases_keep_fixed_names_and_exact_nonclosure_statuses(self):
        self.assertEqual(len(ATTENTION_CASES), 6)
        self.assertEqual(dict(full_import.ATTENTION_CASE_TESTS), ATTENTION_CASES)
        for case, test_name in ATTENTION_CASES.items():
            with self.subTest(case=case):
                self.assertEqual(full_import.CASE_TESTS[case], test_name)
                self.assertEqual(runner.ALL_CASE_TESTS[case], test_name)
                expected = ('FULL_RESTORE_REHEARSAL_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'
                            if case == 'attention-full-chain' else
                            'EXPECTED_REJECTION_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE')
                status = runner.full_case_pass_status(case)
                self.assertEqual(status, expected)
                self.assertNotIn(status, (runner.FULL_PASS, runner.RECOVERY_PASS, runner.SNAPSHOT_PASS))

    def test_four_fixed_domains_are_disjoint_and_close_the_full_case_roster(self):
        domains = {
            'original_full': set(ORIGINAL_FULL_CASES),
            'recovery': set(full_import.RECOVERY_CASE_TESTS),
            'snapshot': set(full_import.SNAPSHOT_CASE_TESTS),
            'attention': set(ATTENTION_CASES),
        }
        self.assertEqual({name: len(cases) for name, cases in domains.items()},
                         dict(original_full=9, recovery=5, snapshot=11, attention=6))
        for (left, left_cases), (right, right_cases) in combinations(domains.items(), 2):
            with self.subTest(left=left, right=right):
                self.assertTrue(left_cases.isdisjoint(right_cases))
        self.assertEqual(set(full_import.CASE_TESTS), set().union(*domains.values()))
        self.assertEqual(len(full_import.CASE_TESTS), 31)

    def test_eleven_snapshot_cases_keep_fixed_names_and_their_nonclosure_status(self):
        prefix = ('restore_preflight::target_binding::controlled_import::full::'
                  'snapshot_effects_tests::recovery_snapshot_')
        expected = {
            'snapshot-p1': prefix + 'p1_revoke_unexpired',
            'snapshot-p2': prefix + 'p2_revoke_full_history',
            'snapshot-p3-live': prefix + 'p3_ceiling_unexpired',
            'snapshot-p3-expired': prefix + 'p3_ceiling_expired',
            'snapshot-p4': prefix + 'p4_real_none_after_third_expiry',
            'snapshot-p5-one': prefix + 'p5_transient_one',
            'snapshot-p5-two': prefix + 'p5_transient_two',
            'snapshot-p5-terminal': prefix + 'p5_terminal_unchanged',
            'snapshot-p6': prefix + 'p6_committed_copy_held_delivery',
            'snapshot-queued-zero': prefix + 'queued_zero_no_effect',
            'snapshot-history-gap': prefix + 'direct_claim_history_gap_never_positive',
        }
        self.assertEqual(dict(full_import.SNAPSHOT_CASE_TESTS), expected)
        self.assertTrue(set(expected).isdisjoint(full_import.RECOVERY_CASE_TESTS))
        for case, test_name in expected.items():
            with self.subTest(case=case):
                self.assertEqual(full_import.CASE_TESTS[case], test_name)
                status = runner.full_case_pass_status(case)
                self.assertEqual(status,
                    'SNAPSHOT_EFFECT_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_TASK6_COMPLETE')
                self.assertNotIn(status, (runner.FULL_PASS, runner.RECOVERY_PASS))

    def test_an_arbitrary_recovery_prefix_does_not_select_a_case(self):
        with self.assertRaises(runner.CompletionError):
            runner.full_case_pass_status('recovery-unreviewed')

    def test_an_arbitrary_snapshot_prefix_does_not_select_a_case(self):
        for case in ('snapshot-unreviewed', 'snapshot-p1-unreviewed',
                     'snapshot-p3', 'snapshot-P6', 'snapshot-history-gap-extra'):
            with self.subTest(case=case), self.assertRaises(runner.CompletionError):
                runner.full_case_pass_status(case)

    def test_arbitrary_or_malicious_attention_names_do_not_select_a_case(self):
        invalid = ['attention-unreviewed', 'attention-', 'Attention-full-chain',
                   'attention-full-chain ', 'attention-full-chain/../full-import',
                   'attention-full-chain;full-import', 'attention-FULL-CHAIN']
        for case, test_name in ATTENTION_CASES.items():
            invalid.extend((case + '-unreviewed', 'unreviewed-' + case,
                            case.replace('attention-', 'attention_', 1), test_name))
        for case in invalid:
            with self.subTest(case=case), self.assertRaises(runner.CompletionError):
                runner.full_case_pass_status(case)


if __name__ == '__main__':
    unittest.main()
