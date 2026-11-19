"""Application CI requires a complete passing suite, with no failure allowance."""
import unittest

from run_tests import accepts_suite


LOG = """   Finished `test` profile [optimized + debuginfo] target(s) in 0.24s
     Running unittests src/main.rs (app/target/debug/deps/recordbox-123)

running 3 tests
test backup::tests::backup_restore_round_trip ... ok
test export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic ... ok
test export::usb_test::recover_real_device ... ignored, requires a manually supplied device

test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.00s

"""


class ApplicationSuiteTests(unittest.TestCase):
    def test_complete_passing_suite_and_explicit_ignored_tests_are_accepted(self):
        self.assertTrue(accepts_suite(LOG, 0))
        self.assertTrue(accepts_suite(LOG.replace(", requires a manually supplied device", ""), 0))
        all_passed = LOG.replace("ignored, requires a manually supplied device", "ok")
        all_passed = all_passed.replace("2 passed; 0 failed; 1 ignored", "3 passed; 0 failed; 0 ignored")
        self.assertTrue(accepts_suite(all_passed, 0))

    def test_expected_panic_test_uses_valid_libtest_output(self):
        self.assertTrue(accepts_suite(LOG.replace(
            "backup::tests::backup_restore_round_trip ... ok",
            "backup::tests::rejects_invalid_input - should panic ... ok"), 0))

    def test_no_failures_are_accepted_including_former_pdb_exception(self):
        for name in ("backup::tests::backup_restore_round_trip",
                     "export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic"):
            log = LOG.replace(f"test {name} ... ok", f"test {name} ... FAILED")
            log = log.replace("test result: ok. 2 passed; 0 failed", "test result: FAILED. 1 passed; 1 failed")
            for code in (0, 101):
                with self.subTest(name=name, code=code):
                    self.assertFalse(accepts_suite(log, code))

    def test_crash_or_error_exit_rejects_even_a_passing_summary(self):
        for code in (-11, 1, 101):
            with self.subTest(code=code):
                self.assertFalse(accepts_suite(LOG, code))
        self.assertFalse(accepts_suite(LOG + "error: could not compile recordbox\n", 0))

    def test_filtered_measured_or_empty_suite_is_rejected(self):
        for old, new in (("0 filtered out", "1 filtered out"),
                         ("0 measured", "1 measured"),
                         ("2 passed", "0 passed")):
            with self.subTest(new=new):
                self.assertFalse(accepts_suite(LOG.replace(old, new), 0))

    def test_missing_or_truncated_runner_output_is_rejected(self):
        self.assertFalse(accepts_suite(LOG.split("test result:")[0], 0))
        self.assertFalse(accepts_suite(LOG.replace("running 3 tests\n", ""), 0))
        self.assertFalse(accepts_suite(LOG.replace("running 3 tests", "running 4 tests"), 0))
        self.assertFalse(accepts_suite(LOG.replace("test backup::tests::backup_restore_round_trip ... ok\n", ""), 0))
        self.assertFalse(accepts_suite(LOG.replace("; finished in 1.00s", ""), 0))

    def test_forged_duplicate_or_mismatched_results_are_rejected(self):
        self.assertFalse(accepts_suite(LOG.replace(
            "backup::tests::backup_restore_round_trip",
            "export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic"), 0))
        self.assertFalse(accepts_suite(LOG.replace("2 passed; 0 failed; 1 ignored", "1 passed; 0 failed; 2 ignored"), 0))
        self.assertFalse(accepts_suite(LOG + LOG, 0))


if __name__ == "__main__":
    unittest.main()
