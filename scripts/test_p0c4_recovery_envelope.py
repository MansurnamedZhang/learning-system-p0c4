"""Task6 foundation results cannot inherit Task5 or full-closure credit."""
import unittest
from scripts import p0c4_completion_acceptance as runner
from scripts.p0c4_completion import full_import


class RecoveryEnvelopeTests(unittest.TestCase):
    def test_all_five_recovery_cases_have_an_explicit_nonclosure_result(self):
        self.assertEqual(len(full_import.RECOVERY_CASE_TESTS), 5)
        for case in full_import.RECOVERY_CASE_TESTS:
            self.assertEqual(runner.full_case_pass_status(case),
                'RECOVERY_FOUNDATION_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_TASK6_COMPLETE')
            self.assertNotEqual(runner.full_case_pass_status(case), runner.FULL_PASS)

    def test_existing_nine_full_cases_keep_their_original_status(self):
        cases = (set(full_import.CASE_TESTS) - set(full_import.RECOVERY_CASE_TESTS)
                 - set(full_import.SNAPSHOT_CASE_TESTS))
        self.assertEqual(len(cases), 9)
        for case in cases:
            self.assertEqual(runner.full_case_pass_status(case), runner.FULL_PASS)

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


if __name__ == '__main__':
    unittest.main()
