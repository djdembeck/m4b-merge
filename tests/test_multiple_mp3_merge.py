from m4b_merge import audible_helper, config, m4b_helper, helpers
import os
import shutil
from pathlib import Path
import pytest
import subprocess

# Test with Project Haill Mary, because it's a good book
primary_asin = "B08G9PRS1K"

# Test mp3 and it's full path
test_path = Path("tests/media_files/multi_mp3")

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
    make_mp3 = create_blank_audio()
    # Run all tests
    yield make_mp3
    # After all tests have run:
    shutil.rmtree(output_dir)


class TestMerge:
    # Test that get_directory works on directory with multiple files
    def test_get_directory_with_files(self):
        input_data = helpers.get_directory(test_path)
        assert input_data == (test_path, 'mp3', 2)

    def test_download_cover(self):
        mp3 = self.mp3_data(primary_asin)
        mp3.download_cover()
        assert ((test_cover).exists() and
                os.path.getsize(test_cover) == 779312)

    def test_bitrate(self):
        mp3 = self.mp3_data(primary_asin)
        first_file = mp3.find_file_for_mediainfo()
        bitrate = mp3.find_bitrate(first_file)
        assert bitrate == 128000

    def test_samplerate(self):
        mp3 = self.mp3_data(primary_asin)
        first_file = mp3.find_file_for_mediainfo()
        samplerate = mp3.find_samplerate(first_file)
        assert samplerate == 44100

    def test_merge(self):
        mp3 = self.mp3_data(primary_asin)
        mp3.prepare_data()
        mp3.prepare_command_args()
        mp3.merge_multiple_files()
        assert (output_path.exists() and
                os.path.getsize(output_path) == 25300169 or 25329629)

    def test_chapter_generation(self):
        mp3 = self.mp3_data(primary_asin)
        mp3.prepare_data()
        mp3.prepare_command_args()
        mp3.fix_chapters()
        assert (output_chapters.exists() and
                os.path.getsize(output_chapters) == 76)

    def mp3_data(self, asin):
        input_data = helpers.get_directory(test_path)
        aud = audible_helper.BookData(asin)
        metadata = aud.fetch_api_data(config.api_url)
        chapters = aud.get_chapters()

        # Process metadata and run components to merge files
        mp3 = m4b_helper.M4bMerge(input_data, metadata, test_path, chapters)
        return mp3


# Create blank audio for testing
def create_blank_audio():
    Path(test_path).mkdir(
            parents=True,
            exist_ok=True
    )
    test_files = ['1.mp3', '2.mp3']
    ffmpegargs = [
        'ffmpeg',
        '-f',
        'lavfi',
        '-t',
        '29127',
        '-i',
        'anullsrc=cl=stereo:r=44100',
    ]
    for file in test_files:
        this_path = Path(test_path, file)
        if not this_path.exists():
            print("Generating empty audio file for testing...")
            ffmpegargs.append(this_path)
            subprocess.run(ffmpegargs, stdout=subprocess.PIPE)
            ffmpegargs.pop()
