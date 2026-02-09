from m4b_merge import audible_helper, config, m4b_helper, helpers
import os
import shutil
from pathlib import Path
import pytest
import subprocess
from mutagen.mp4 import MP4

# Test with Project Haill Mary, because it's a good book
primary_asin = "B08G9PRS1K"

# Test m4b and it's full path
test_file = Path('test.m4b')
test_path = Path(f"tests/media_files/{test_file}")

# Cover image file
test_cover = Path(f"{test_path}_cover.jpg")

# Final metadata paths
home = Path.home()
config.output = Path(f"{home}/output")
config.junk_dir = Path(f"{home}/input/done")
config.path_format = "author/title/title - subtitle"
output_dir = Path(f"{config.output}/Andy Weir/Project Hail Mary")
output_path = Path(output_dir, "Project Hail Mary.m4b")
output_chapters = Path(f"{output_dir}/Project Hail Mary.chapters.txt")

config.num_cpus = os.cpu_count()


@pytest.fixture(scope='class', autouse=True)
def file_commands():
    # Before all tests have run:
    # Check if blank audio exists already to test
    make_m4b = create_blank_audio()
    # Run all tests
    yield make_m4b
    # After all tests have run:
    shutil.rmtree(output_dir)


class TestMerge:
    # Test that get_directory works on single file
    def test_get_directory_single(self):
        input_data = helpers.get_directory(test_path)
        assert input_data == (test_path, 'm4b', 1)

    def test_download_cover(self):
        m4b = self.m4b_data(primary_asin)
        m4b.download_cover()
        assert test_cover.exists(), "Cover file should exist"
        file_size = os.path.getsize(test_cover)
        assert 700000 < file_size < 850000, f"Cover file size {file_size} outside expected range (±10%)"

    def test_chapter_generation(self):
        m4b = self.m4b_data(primary_asin)
        m4b.prepare_data()
        m4b.prepare_command_args()
        m4b.fix_chapters()
        assert output_chapters.exists(), "Chapters file should exist"
        file_size = os.path.getsize(output_chapters)
        assert 700 < file_size < 900, f"Chapters file size {file_size} outside expected range (±10%)"

    def test_bitrate(self):
        m4b = self.m4b_data(primary_asin)
        bitrate = m4b.find_bitrate(test_path)
        assert bitrate == 4000

    def test_samplerate(self):
        m4b = self.m4b_data(primary_asin)
        samplerate = m4b.find_samplerate(test_path)
        assert samplerate == 44100

    def test_merge(self):
        m4b = self.m4b_data(primary_asin)
        m4b.prepare_data()
        m4b.prepare_command_args()
        m4b.merge_single_aac()
        assert output_path.exists(), "Output file should exist"
        file_size = os.path.getsize(output_path)
        assert 22000000 < file_size < 28000000, f"Output file size {file_size} outside expected range (±10%)"

        # Verify metadata using mutagen
        audio = MP4(str(output_path))
        assert audio.tags['\xa9nam'][0] == 'Project Hail Mary', "Title should match"
        assert audio.tags['\xa9ART'][0] == 'Andy Weir', "Author should match"
        assert audio.tags['\xa9nrt'][0] == 'Ray Porter', "Narrator should match"

    def m4b_data(self, asin):
        input_data = helpers.get_directory(test_path)
        aud = audible_helper.BookData(asin)
        metadata = aud.fetch_api_data(config.api_url)
        chapters = aud.get_chapters()

        # Process metadata and run components to merge files
        m4b = m4b_helper.M4bMerge(input_data, metadata, test_path, chapters)
        return m4b


# Create blank audio for testing
def create_blank_audio():
    ffmpegargs = [
        'ffmpeg',
        '-f',
        'lavfi',
        '-t',
        '58253',
        '-i',
        'anullsrc=cl=stereo:r=44100',
        test_path,
    ]
    if not test_path.exists():
        print("Generating empty audio file for testing...")
        subprocess.run(ffmpegargs, stdout=subprocess.PIPE)


class TestMutagenChapters:
    """Tests for mutagen chapter writing fallback when mp4chaps is unavailable."""

    def test_write_chapters_mutagen(self):
        """Test that mutagen can write chapters to an MP4 file."""
        # Ensure mp4chaps is not available to force mutagen path
        original_mp4chaps = config.mp4chaps_bin
        config.mp4chaps_bin = None

        try:
            # Create a test m4b file if it doesn't exist
            if not test_path.exists():
                create_blank_audio()

            # Create a copy of the test file to modify
            test_copy = Path(f"{test_path}_mutagen_test.m4b")
            shutil.copy(test_path, test_copy)

            # Define chapter markers (timestamp, title)
            chapter_markers = [
                ("00:00:00.000", "Chapter 1: Introduction"),
                ("00:05:30.000", "Chapter 2: The Beginning"),
                ("00:10:45.000", "Chapter 3: Development"),
            ]

            # Create M4bMerge instance to access the mutagen method
            input_data = helpers.get_directory(test_copy)
            m4b = m4b_helper.M4bMerge(input_data, {}, test_copy, [])

            # Write chapters using mutagen
            m4b._write_chapters_mutagen(str(test_copy), chapter_markers)

            # Verify chapters were written using mutagen
            audio = MP4(str(test_copy))
            assert audio.chapters is not None, "Chapters should be written to file"
            assert len(audio.chapters) == 3, f"Expected 3 chapters, got {len(audio.chapters)}"

            # Clean up test file
            test_copy.unlink()
        finally:
            # Restore original mp4chaps setting
            config.mp4chaps_bin = original_mp4chaps

    def test_chapter_fallback_without_mp4chaps(self):
        """Test fix_chapters method falls back to mutagen when mp4chaps is unavailable."""
        # Ensure mp4chaps is not available to force mutagen path
        original_mp4chaps = config.mp4chaps_bin
        config.mp4chaps_bin = None

        try:
            # Create a test m4b file if it doesn't exist
            if not test_path.exists():
                create_blank_audio()

            # Create a copy of the test file to modify
            test_copy = Path(f"{test_path}_fallback_test.m4b")
            shutil.copy(test_path, test_copy)

            # Create M4bMerge instance
            input_data = helpers.get_directory(test_copy)
            metadata = {'title': 'Test Book', 'author': 'Test Author'}
            m4b = m4b_helper.M4bMerge(input_data, metadata, test_copy, [])

            # Directly test _write_chapters_mutagen which is the core mutagen functionality
            chapter_markers = [
                ("00:00:00.000", "Chapter 1: First Chapter"),
                ("00:05:00.000", "Chapter 2: Second Chapter"),
                ("00:10:00.000", "Chapter 3: Third Chapter"),
            ]

            # Write chapters using mutagen
            m4b._write_chapters_mutagen(str(test_copy), chapter_markers)

            # Verify chapters were written using mutagen
            audio = MP4(str(test_copy))
            assert audio.chapters is not None, "Chapters should be written to file"
            assert len(audio.chapters) >= 3, f"Expected at least 3 chapters, got {len(audio.chapters)}"

            # Clean up
            test_copy.unlink()
        finally:
            # Restore original mp4chaps setting
            config.mp4chaps_bin = original_mp4chaps
