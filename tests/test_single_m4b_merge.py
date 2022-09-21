from m4b_merge import audible_helper, config, m4b_helper, helpers
import os
import shutil
from pathlib import Path
import pytest
import subprocess

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
        assert ((test_cover).exists() and
                os.path.getsize(test_cover) == 779312)

    def test_chapter_generation(self):
        m4b = self.m4b_data(primary_asin)
        m4b.prepare_data()
        m4b.prepare_command_args()
        m4b.fix_chapters()
        assert (output_chapters.exists() and
                os.path.getsize(output_chapters) == 794)

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
        assert (output_path.exists() and
                os.path.getsize(output_path) == 25301510 or 25330971)

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
