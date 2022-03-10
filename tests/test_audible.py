import datetime
from m4b_merge import audible_helper, config, helpers
import pytest

# Test with Project Haill Mary, because it's a good book
primary_asin = "B08G9PRS1K"
config.api_url = "https://api.audnex.us"


class TestASINValidation:
    # Call ASIN validator
    def audible_request(self, asin):
        helpers.validate_asin(config.api_url, asin)

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
        aud.fetch_api_data(config.api_url)
        chapters = aud.get_chapters()
        return chapters

    # Test named chapters returning with timestamp
    def test_chapter_name(self):
        chapters = self.chapters(primary_asin)
        assert chapters[1] == "0:00:00.000 Opening Credits"

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
        return aud.fetch_api_data(config.api_url)

    def test_single_author_single_narrator(self):
        errors = []
        metadata = self.audible_data(primary_asin)
        # Check title
        if metadata['title'] != "Project Hail Mary":
            errors.append("Error with title")
        # Check author
        if metadata['authors'][0]['name'] != "Andy Weir":
            errors.append("Error with author name")
        if metadata['authors'][0]['asin'] != "B00G0WYW92":
            errors.append("Error with author ASIN")
        # Check narrator
        if metadata['narrators'][0]['name'] != "Ray Porter":
            errors.append("Error with narrator")
        # Check release date object
        if not isinstance(
            datetime.datetime.fromisoformat(
                metadata['releaseDate'].replace('Z', '+00:00')
            ), datetime.date
        ):
            errors.append("Error with release date")
        # Genres/tags
        if len(metadata['genres']) != 5:
            errors.append("Not enough genres")
        if metadata['genres'][0]['name'] != "Science Fiction & Fantasy":
            errors.append("Genre 1 is incorrect")
        if metadata['genres'][0]['type'] != "genre":
            errors.append("Genre  type is incorrect")
        if metadata['genres'][1]['name'] != "Science Fiction":
            errors.append("Genre 2 is incorrect")
        if metadata['genres'][1]['type'] != "genre":
            errors.append("Genre 2 type is incorrect")
        if metadata['genres'][2]['name'] != "Adventure":
            errors.append("Genre 2 is incorrect")
        if metadata['genres'][2]['type'] != "tag":
            errors.append("Genre 2 type is incorrect")
        if metadata['genres'][3]['name'] != "Hard Science Fiction":
            errors.append("Genre 2 is incorrect")
        if metadata['genres'][3]['type'] != "tag":
            errors.append("Genre 2 type is incorrect")
        if metadata['genres'][4]['name'] != "Space Opera":
            errors.append("Genre 2 is incorrect")
        if metadata['genres'][4]['type'] != "tag":
            errors.append("Genre 2 type is incorrect")
        # Check publisher name
        if metadata['publisherName'] != "Audible Studios":
            errors.append("Error with publisher")
        # Check language
        if metadata['language'] != "english":
            errors.append("Error with language")
        # Image
        if not metadata['image']:
            errors.append("No cover image found")
        # Assert no errors come back
        assert not errors, "Errors occured:\n{}".format("\n".join(errors))

    def test_multiple_author_multiple_narrator(self):
        errors = []
        metadata = self.audible_data("B08C6YJ1LS")
        # Check title
        if metadata['title'] != "The Coldest Case: A Black Book Audio Drama":
            errors.append("Error with title")
        # Check author
        if metadata['authors'][2]['name'] != "Ryan Silbert":
            errors.append("Error with author name")
        if metadata['authors'][2]['asin'] != "B07R2F2DXH":
            errors.append("Error with author ASIN")
        # Check narrator
        if metadata['narrators'][1]['name'] != "Krysten Ritter":
            errors.append("Error with narrator")
        # Check release date object
        if not isinstance(
            datetime.datetime.fromisoformat(
                metadata['releaseDate'].replace('Z', '+00:00')
            ), datetime.date
        ):
            errors.append("Error with release date")
        # Genres/tags
        if len(metadata['genres']) < 2:
            errors.append("Not enough genres")
        if metadata['genres'][0]['name'] != "Mystery, Thriller & Suspense":
            errors.append("Genre 1 is incorrect")
        if metadata['genres'][0]['type'] != "genre":
            errors.append("Genre  type is incorrect")
        if metadata['genres'][1]['name'] != "Thriller & Suspense":
            errors.append("Genre 2 is incorrect")
        if metadata['genres'][1]['type'] != "genre":
            errors.append("Genre 2 type is incorrect")
        # Check publisher name
        if metadata['publisherName'] != "Audible Originals":
            errors.append("Error with publisher")
        # Check language
        if metadata['language'] != "english":
            errors.append("Error with language")
        # Image
        if not metadata['image']:
            errors.append("No cover image found")
        # Series
        if metadata['seriesPrimary']['asin'] != "B08RLSPY4J":
            errors.append("Series asin incorrect")
        if metadata['seriesPrimary']['name'] != "A Billy Harney Thriller":
            errors.append("Series name incorrect")
        if metadata['seriesPrimary']['position'] != "0.5":
            errors.append("Series position incorrect")
        # Assert no errors come back
        assert not errors, "Errors occured:\n{}".format("\n".join(errors))
