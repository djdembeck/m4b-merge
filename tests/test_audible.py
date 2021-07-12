import datetime
from m4b_merge import audible_helper, helpers
import pytest

# Test with Project Haill Mary, because it's a good book
primary_asin = "B08G9PRS1K"


class TestASINValidation:
    # Call ASIN validator
    def audible_request(self, asin):
        helpers.validate_asin(asin)

    # Check a known good ASIN
    def test_audible_request_valid(self):
        assert self.audible_request(primary_asin) is None

    # Check a blank asin
    def test_audible_request_blank(self):
        with pytest.raises(ValueError):
            self.audible_request("")

    # Check a known bad ASIN
    def test_audible_request_invalid(self):
        with pytest.raises(ValueError):
            self.audible_request("1234567891")


class TestChapterData:
    def audible_data(self, asin):
        aud = audible_helper.BookData(asin)
        return aud

    # Create new Audible object to work with
    def chapters(self, asin):
        aud = self.audible_data(asin)
        chapters = aud.get_chapters()
        return chapters

    # Test named chapters returning with timestamp
    def test_chapter_name(self):
        chapters = self.chapters(primary_asin)
        assert chapters[1] == "0:00:00 Opening Credits"

    def test_chapter_int_with_period(self):
        # Use ASIN that has periods in chapter name
        asin = "1721358595"
        chapters = self.chapters(asin)
        # Assert that last chapter equals known data
        assert chapters[5].split(' ', 1)[1] == "Chapter 1"

    # Test long durations with War & Peace, which is 55.5 hours long
    long_duration_asin = "1799744698"

    # Verify timestamps are being formatted correctly
    def test_timestamp_conversion(self):
        chapters = self.chapters(self.long_duration_asin)
        # Assert that last chapter equals known data
        assert chapters[len(chapters) - 1].split(' ')[0] == "55:17:58.186"


class TestMetadata:
    def audible_data(self, asin):
        aud = audible_helper.BookData(asin)
        return aud

    def test_returned_data(self):
        errors = []
        metadata = self.audible_data(primary_asin).parser()
        # Check title
        if metadata['title'] != "Project Hail Mary":
            errors.append("Error with title")
        # Check author
        if metadata['authors'][0]['name'] != "Andy Weir":
            errors.append("Error with author")
        # Check narrator
        if metadata['narrators'][0] != "Ray Porter":
            errors.append("Error with narrator")
        # Check release date object
        if not isinstance(metadata['release_date'], datetime.date):
            errors.append("Error with release date")
        # Check publisher name
        if metadata['publisher_name'] != "Audible Studios":
            errors.append("Error with publisher")
        # Check language
        if metadata['language'] != "english":
            errors.append("Error with language")
        if not metadata['cover_image']:
            errors.append("No cover image found")
        # Assert no errors come back
        assert not errors, "Errors occured:\n{}".format("\n".join(errors))
