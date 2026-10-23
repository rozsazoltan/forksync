"""Unit tests for CalVer tag parsing and month rollover; no GitHub needed."""
import unittest
from datetime import datetime, timezone
from calver import next_version, VALID


class CalverTests(unittest.TestCase):
    def test_first_release_starts_at_zero(self):
        self.assertEqual(next_version([], datetime(2026, 10, 23, tzinfo=timezone.utc)), '2026.10.0')

    def test_increments_highest_number_not_tag_count(self):
        self.assertEqual(next_version(['v2026.10.0', 'v2026.10.9', 'v2026.10.3'], datetime(2026, 10, 25, tzinfo=timezone.utc)), '2026.10.10')

    def test_new_month_resets_revision(self):
        self.assertEqual(next_version(['v2026.10.16'], datetime(2026, 11, 2, tzinfo=timezone.utc)), '2026.11.0')

    def test_ignores_unrelated_and_malformed_tags(self):
        self.assertEqual(next_version(['v2026.010.1', 'v2026.10.01', 'release-2026.10.2', 'v2025.10.88'], datetime(2026, 10, 23, tzinfo=timezone.utc)), '2026.10.0')

    def test_release_tag_validation(self):
        self.assertIsNotNone(VALID.fullmatch('v2026.10.0'))
        self.assertIsNone(VALID.fullmatch('v2026.10.23.1'))
        self.assertIsNone(VALID.fullmatch('v2026.00.0'))


if __name__ == '__main__':
    unittest.main()
