from m4b_merge import audible_helper, helpers
import pytest


class TestAudibleHTTP:
    # Call ASIN validator
    def audible_request(self, asin):
        helpers.validate_asin(asin)

    # Check a known good ASIN
    def test_audible_request_valid(self):
        assert self.audible_request("B08G9PRS1K") is None

    # Check a blank asin
    def test_audible_request_blank(self):
        with pytest.raises(ValueError):
            self.audible_request("")

    # Check a known bad ASIN
    def test_audible_request_invalid(self):
        with pytest.raises(ValueError):
            self.audible_request("1234567891")


# TODO: Check the title, author and other data used in m4b_merge.
