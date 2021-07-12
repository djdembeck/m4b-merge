import logging
import math
import os
import requests
import shutil
import subprocess
from pathlib import Path
from pathvalidate import sanitize_filename
from pydub.utils import mediainfo
# Local imports
from . import config, helpers


class M4bMerge:
    def __init__(self, input_data, metadata, chapters=None):
        self.input_path = input_data[0]
        self.input_extension = input_data[1]
        self.num_of_files = input_data[2]
        self.metadata = metadata
        self.chapters = chapters

    def download_cover(self):
        if 'cover_image' in self.metadata:
            # Request to image URL
            cover_request = requests.get(self.metadata['cover_image'])
            # Verify image exists
            if cover_request.status_code == 200:
                # Path to write image to
                self.cover_path = f"{self.input_path}_cover.jpg"
                # Write image
                with open(self.cover_path, 'wb') as f:
                    f.write(cover_request.content)
            else:
                logging.error("Couldn't download Audible cover")
        else:
            logging.warning("No cover image available from Audible")

    def prepare_data(self):
        # Metadata variables
        # Only use subtitle in case of metadata, not file name
        if 'subtitle' in self.metadata:
            base_title = self.metadata['title']
            base_subtitle = self.metadata['subtitle']
            title = f"{base_title} - {base_subtitle}"
        else:
            title = self.metadata['title']
        # Only use first author/narrator for file names;
        # no subtitle for file name
        path_title = self.metadata['title']
        path_author = self.metadata['authors'][0]['name']
        # For embedded, use all authors/narrators
        author_name_arr = []
        for authors in self.metadata['authors']:
            author_name_arr.append(authors['name'])
        author = ', '.join(author_name_arr)
        narrator = ', '.join(self.metadata['narrators'])
        if 'series' in self.metadata:
            series = self.metadata['series']
        else:
            series = None
        summary = self.metadata['short_summary']
        year = self.metadata['release_date'].year

        self.book_output = (
            f"{config.output}/{sanitize_filename(path_author)}/"
            f"{sanitize_filename(path_title)}"
        )
        self.file_title = sanitize_filename(title)

        # Download cover image
        self.download_cover()
        ##

        # Make necessary directories
        # Final output folder
        Path(self.book_output).mkdir(
            parents=True,
            exist_ok=True
        )

        # Folder to move original input into
        Path(config.junk_dir).mkdir(
            parents=True,
            exist_ok=True
        )

        # Array for argument use
        # main metadata args
        self.metadata_args = [
            f"--name={title}",
            f"--album={path_title}",
            f"--artist={narrator}",
            f"--albumartist={author}",
            f"--year={year}",
            f"--description={summary}"
        ]

        # Append series to metadata if it exists
        if series:
            self.metadata_args.append(f"--series={series}")

        if self.cover_path:
            self.metadata_args.append(f"--cover={self.cover_path}")

        # args for merge  process
        self.processing_args = [
            '--force',
            '--no-chapter-reindexing',
            '--no-cleanup',
            f'--jobs={config.num_cpus}'
        ]

        # Set logging level of m4b-tool depending upon log_level
        level = logging.root.level
        if level == logging.INFO:
            self.processing_args.append('-v')
        elif level == logging.DEBUG:
            self.processing_args.append('-vvv')

    def find_bitrate(self, file_input):
        # Divide bitrate by 1k, round up,
        # and return back to 1k divisible for round number.
        target_bitrate = math.ceil(
            int(mediainfo(file_input)['bit_rate']) / 1000
        ) * 1000
        logging.info(f"Source bitrate: {target_bitrate}")

        return target_bitrate

    def find_samplerate(self, file_input):
        target_samplerate = int(
            mediainfo(
                file_input
            )['sample_rate']
        )
        logging.info(f"Source samplerate: {target_samplerate}")

        return target_samplerate

    def run_merge(self):
        # Prepare metadata commands
        self.prepare_data()

        # Handle multiple input files in a folder
        if ((self.input_path.is_dir() and self.num_of_files > 1)
                or self.input_extension is None):
            self.merge_multiple_files()
        # Handle single m4b or m4a input file
        elif ((self.input_path.is_file()) and
                (self.input_extension == "m4b" or
                    self.input_extension == "m4a")):
            self.merge_single_aac()
        # Handle single mp3 input file
        elif self.input_path.is_file() and self.input_extension == "mp3":
            self.merge_single_mp3()
        # If none of the above are true, log error
        else:
            logging.error(
                f"Couldn't determine input type/extension for"
                f" {self.file_title}")

    def merge_multiple_files(self):
        logging.info("Processing multiple files in a dir...")

        # If multi-disc, find the extension
        if not self.input_extension:
            input_path_glob = Path(self.input_path).glob('**/*')
            i = 0
            sorted_multi = sorted(input_path_glob)
            while not sorted_multi[i].is_dir():
                logging.debug("Looking for first dir in multi-dir...")
                i += 1
            selected_input = sorted_multi[i]
            logging.debug(
                f"Result was #{i+1} for first dir: {selected_input}"
            )
            self.input_extension = helpers.find_extension(
                selected_input)[1]
            logging.debug(
                (f"Guessed multi-disc extension to be:"
                    f" {self.input_extension}")
            )
        else:
            selected_input = self.input_path

        selected_input_glob = Path(selected_input).glob('**/*')

        # Find first file with our extension, to check rates against
        for file in sorted(selected_input_glob):
            if file.suffix == f".{self.input_extension}":
                first_file = file
                break

        logging.debug(f"Got file to run mediainfo on: {first_file}")

        # Mediainfo data
        target_bitrate = self.find_bitrate(first_file)
        target_samplerate = self.find_samplerate(first_file)
        ##

        args = [
            config.m4b_tool_bin,
            'merge',
            f"--output-file={self.book_output}/{self.file_title}.m4b"
        ]

        # Add in main metadata and merge args
        args.extend(self.metadata_args)
        args.extend(self.processing_args)

        if self.input_extension == "m4b" or self.input_extension == "m4a":
            logging.info(
                f"Multiple {self.input_extension} files, not converting"
            )
            args.append("--no-conversion")
        else:
            args.append(f"--audio-bitrate={target_bitrate}")
            args.append(f"--audio-samplerate={target_samplerate}")

        # Append input path
        args.append(self.input_path)

        # m4b command with passed args
        logging.debug(f"M4B command: {args}")
        subprocess.call(args, shell=False)

        # Move obsolete input to processed folder
        self.move_completed_input()
        # Process chapters
        self.fix_chapters()

    def merge_single_aac(self):
        logging.info(f"Processing single {self.input_extension} input...")

        args = [
            config.m4b_tool_bin,
            'meta',
            (f"{self.input_path.parent}/"
                f"{self.input_path.stem}.new.m4b")
        ]
        # Add in main metadata args
        args.extend(self.metadata_args)

        # make backup file
        shutil.copy(
            self.input_path,
            f"{self.input_path.parent}/{self.input_path.stem}.new.m4b"
            )

        # m4b command with passed args
        logging.debug(f"M4B command: {args}")
        subprocess.call(args, shell=False)

        # Move completed file
        shutil.move(
            f"{self.input_path.parent}/{self.input_path.stem}.new.m4b",
            f"{self.book_output}/{self.file_title}.m4b"
        )

        # Move obsolete input to processed folder
        self.move_completed_input()
        # Process chapters
        self.fix_chapters()

    def merge_single_mp3(self):
        logging.info(f"Processing single {self.input_extension} input...")

        # Mediainfo data
        target_bitrate = self.find_bitrate(self.input_path)
        target_samplerate = self.find_samplerate(self.input_path)
        ##

        args = [
            config.m4b_tool_bin,
            'merge',
            f"--output-file={self.book_output}/{self.file_title}.m4b",
            f"--audio-bitrate={target_bitrate}",
            f"--audio-samplerate={target_samplerate}"
        ]
        # Add in main metadata and merge args
        args.extend(self.metadata_args)
        args.extend(self.processing_args)

        # Append input path
        args.append(self.input_path)

        # m4b command with passed args
        logging.debug(f"M4B command: {args}")
        subprocess.call(args, shell=False)

        # Move obsolete input to processed folder
        self.move_completed_input()
        # Process chapters
        self.fix_chapters()

    def fix_chapters(self):
        chapter_file = f"{self.book_output}/{self.file_title}.chapters.txt"
        m4b_to_modify = f"{self.book_output}/{self.file_title}.m4b"

        # Use audible chapters if they exist and this isn't an mp3
        if self.chapters and self.input_extension != "mp3":
            logging.info("Using chapter data from Audible")
            new_file_content = ('\n'.join(self.chapters))
        elif self.num_of_files == 1 and self.input_extension == "mp3":
            logging.info("Using chapter data from Audible on single mp3")
            new_file_content = ('\n'.join(self.chapters))
        # Else fix formatting of existing chapters
        else:
            logging.info("Using existing chapter data")
            new_file_content = ""
            with open(chapter_file) as f:
                # Store and then skip past total length section
                for line in f:
                    if "# total-length" in line.strip():
                        new_file_content += line.strip() + "\n"
                        break
                # Iterate over rest of the file
                counter = 0
                for line in f:
                    stripped_line = line.strip()
                    counter += 1
                    new_line = (
                        (stripped_line[0:13]) +
                        f'Chapter {"{0:0=2d}".format(counter)}'
                        )
                    new_file_content += new_line + "\n"

        with open(chapter_file, 'w') as f:
            f.write(new_file_content)

        # Apply fixed chapters to file
        args = [
            config.m4b_tool_bin,
            'meta',
            m4b_to_modify,
            f"--import-chapters={chapter_file}"
        ]
        subprocess.call(args, shell=False)

    def move_completed_input(self):
        # Cleanup cover file
        if self.cover_path:
            os.remove(self.cover_path)
        # Move obsolete input to processed folder
        if Path(self.input_path.parent, 'done') == config.junk_dir:
            logging.debug("Junk dir is direct parent")
            move_dir = self.input_path
        elif Path(self.input_path.parents[1], 'done') == config.junk_dir:
            logging.debug("Junk dir is double parent")
            move_dir = self.input_path.parent
        else:
            logging.warning("Couldn't find junk dir relative to input")
            move_dir = None

        if move_dir:
            shutil.move(
                f"{move_dir}",
                f"{config.junk_dir}"
            )
