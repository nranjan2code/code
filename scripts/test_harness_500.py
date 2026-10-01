import unittest

from harness_500 import execution_status


class ExecutionStatusTests(unittest.TestCase):
    def test_one_successful_test_is_a_pass(self):
        output = "test result: ok. 1 passed; 0 failed; 0 ignored; 9 filtered out"
        self.assertEqual(execution_status(0, output, False), "passed")

    def test_intentionally_ignored_test_is_not_counted_as_a_pass(self):
        output = "test result: ok. 0 passed; 0 failed; 1 ignored; 9 filtered out"
        self.assertEqual(execution_status(0, output, False), "ignored")

    def test_exact_filter_matching_no_test_is_a_failure(self):
        output = "test result: ok. 0 passed; 0 failed; 0 ignored; 10 filtered out"
        self.assertEqual(execution_status(0, output, False), "failed")

    def test_nonzero_exit_is_a_failure(self):
        output = "test result: FAILED. 0 passed; 1 failed; 0 ignored"
        self.assertEqual(execution_status(101, output, False), "failed")

    def test_timeout_takes_precedence(self):
        self.assertEqual(execution_status(124, "", True), "timeout")


if __name__ == "__main__":
    unittest.main()
