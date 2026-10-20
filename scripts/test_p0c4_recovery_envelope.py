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
        cases = set(full_import.CASE_TESTS) - set(full_import.RECOVERY_CASE_TESTS)
        self.assertEqual(len(cases), 9)
        for case in cases:
            self.assertEqual(runner.full_case_pass_status(case), runner.FULL_PASS)

    def test_an_arbitrary_recovery_prefix_does_not_select_a_case(self):
        with self.assertRaises(runner.CompletionError):
            runner.full_case_pass_status('recovery-unreviewed')


if __name__ == '__main__':
    unittest.main()
