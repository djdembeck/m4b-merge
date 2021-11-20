from datetime import datetime
import logging
import math
import os
import re
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
        if 'image' in self.metadata:
            # Request to image URL
            cover_request = requests.get(self.metadata['image'])
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
            self.cover_path = None

    def prepare_data(self):
        # Metadata variables
        # Only use subtitle in case of metadata, not file name
        if 'title' in self.metadata:
            self.title = self.metadata['title']
        else:
            raise ValueError("No title in metadata")

        if 'subtitle' in self.metadata:
            self.subtitle = self.metadata['subtitle']
        else:
            self.subtitle = None

        # Only use first author/narrator for file names;
        self.path_author = self.metadata['authors'][0]['name']

        # For embedded, use all authors/narrators
        author_name_arr = []
        for authors in self.metadata['authors']:
            author_name_arr.append(authors['name'])
        self.author = ', '.join(author_name_arr)

        narrator_name_arr = []
        for narrators in self.metadata['narrators']:
            narrator_name_arr.append(narrators['name'])
        self.narrator = ', '.join(narrator_name_arr)

        if 'seriesPrimary' in self.metadata:
            self.series_name = self.metadata['seriesPrimary']['name']
            if 'position' in self.metadata['seriesPrimary']:
                self.series_position = (
                    self.metadata['seriesPrimary']['position']
                )
            else:
                self.series_position = None
        else:
            self.series_name = None
            self.series_position = None

        self.summary = self.metadata['description']

        # Convert date string into datetime object
        dateObj = datetime.strptime(
            self.metadata['releaseDate'], '%Y-%m-%dT%H:%M:%S.%fZ'
        )
        self.year = dateObj.year

        self.genre = None
        if 'genres' in self.metadata:
            genre_names = []
            for g in self.metadata['genres']:
                genre_names.append(g['name'])
            self.genre = '/'.join(genre_names)

        # Use format type for comment
        if 'formatType' in self.metadata:
            self.comment = self.metadata['formatType'].capitalize()

    def prepare_command_args(self):
        self.prepare_output_path()

        # Download cover image
        self.download_cover()

        # Make necessary directories
        # Final output folder
        Path(os.path.dirname(self.book_output)).mkdir(
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
        combined_title = self.title
        if self.subtitle:
            combined_title = f"{self.title} - {self.subtitle}"
        self.metadata_args = [
            f"--name={combined_title}",
            f"--album={self.title}",
            f"--artist={self.narrator}",
            f"--albumartist={self.author}",
            f"--year={self.year}",
            f"--description={self.summary}",
        ]

        # Append series to metadata if it exists
        if self.series_name:
            self.metadata_args.append(f"--series={self.series_name}")
            if self.series_position:
                self.metadata_args.append(
                    f"--series-part={self.series_position}"
                )

        if self.genre:
            self.metadata_args.append(
                f"--genre={self.genre}"
            )

        if self.comment:
            self.metadata_args.append(
                f"--comment={self.comment}"
            )

        if self.cover_path:
            self.metadata_args.append(
                f"--cover={self.cover_path}"
            )

        # args for merge  process
        self.processing_args = [
            '--force',
            '--no-chapter-reindexing',
            '--no-cleanup',
            '--ignore-source-tags',
            f'--jobs={config.num_cpus}'
        ]

        # Set logging level of m4b-tool depending upon log_level
        level = logging.root.level
        if level == logging.INFO:
            self.processing_args.append('-v')
        elif level == logging.DEBUG:
            self.processing_args.append('-vvv')
    
    def prepare_output_path(self):
        """
            Parses user input for desired output path.

            For example:
            `author/series_name series_position - title: subtitle (year)/author - title (year)`
        """
        # First we need to replace the terms with actual data
        # Author
        self.replace_tag('author', self.path_author)
        # Narrator
        self.replace_tag('narrator', self.narrator[0])
        # Series Name
        self.replace_tag('series_name', self.series_name)
        # Series Position
        self.replace_tag('series_position', self.series_position)
        # Subtitle
        self.replace_tag('subtitle', self.subtitle)
        # Title
        self.replace_tag('title', self.title)
        # Year
        self.replace_tag('year', self.year)

        # Now prepare the actual path
        split_paths = config.path_format.split('/')
        sanitized_path_arr = []
        for section in split_paths:
            sanitized_path_arr.append(sanitize_filename(section))
        final_path = '/'.join(sanitized_path_arr)
        logging.info(f"Final path format: {final_path}")

        self.book_output = (
            f"{config.output}/"
            f"{final_path}"
        )
        logging.info(f"Complete output path: {self.book_output}")

    def replace_tag(self, key, value):
        if key and value:
            config.path_format = re.sub(key, str(value), config.path_format)
        else:
            self.remove_tag(key)

    def remove_tag(self, tag):
        config.path_format = re.sub(tag, '', config.path_format).rstrip().strip('-')

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

        # Prepare arguments for m4b-tool commands
        self.prepare_command_args()

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

        first_file = self.find_file_for_mediainfo()

        # Mediainfo data
        target_bitrate = self.find_bitrate(first_file)
        target_samplerate = self.find_samplerate(first_file)

        args = [
            config.m4b_tool_bin,
            'merge',
            f"--output-file={self.book_output}.m4b"
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
            '--ignore-source-tags',
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
            f"{self.book_output}.m4b"
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

        args = [
            config.m4b_tool_bin,
            'merge',
            f"--output-file={self.book_output}.m4b",
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

    def find_file_for_mediainfo(self):
        # If multi-disc, find the extension
        if not self.input_extension:
            selected_input = self.find_multi_disc_extension()
        else:
            selected_input = self.input_path

        selected_input_glob = Path(selected_input).glob('**/*')

        # Find first file with our extension, to check rates against
        for file in sorted(selected_input_glob):
            if file.suffix == f".{self.input_extension}":
                first_file = file
                break

        logging.debug(f"Got file to run mediainfo on: {first_file}")
        return first_file

    def find_multi_disc_extension(self):
        # Get all directories
        input_path_glob = Path(self.input_path).glob('**/*')
        # Find first directory in sort
        i = 0
        sorted_multi = sorted(input_path_glob)
        while not sorted_multi[i].is_dir():
            logging.debug("Looking for first dir in multi-dir...")
            i += 1
        selected_input = sorted_multi[i]
        logging.debug(
            f"Result was #{i+1} for first dir: {selected_input}"
        )
        # Now that first sorted directory was found, find it's primary ext
        self.input_extension = helpers.find_extension(
            selected_input)[1]
        logging.debug(
            (f"Guessed multi-disc extension to be:"
                f" {self.input_extension}")
        )
        return selected_input

    def fix_chapters(self):
        chapter_file = f"{self.book_output}.chapters.txt"
        m4b_to_modify = f"{self.book_output}.m4b"

        # Use audible chapters if they exist and this isn't an mp3
        if (self.chapters and self.input_extension != "mp3") or (
            self.num_of_files == 1 and self.input_extension == "mp3"
        ):
            logging.info("Using chapter data from Audible")
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

        # Apply chapter arguments/command
        args = [
            config.mp4chaps_bin,
            '-z',
            m4b_to_modify,
        ]

        # Check that chapter file is valid
        # Generally only an issue because of:
        # https://github.com/sandreas/m4b-tool/issues/141
        if os.path.getsize(chapter_file) == 0:
            logging.error("Chapter file is empty, attempting to correct")
            args.append('-r')
        else:
            logging.info("Applying chapters to m4b...")
            args.append('-i')

        # Set logging level of m4bchaps depending upon log_level
        if logging.root.level == logging.DEBUG:
            args.append('-v')
        elif logging.root.level >= logging.WARNING:
            args.append('-q')

        # Apply fixed chapters to file
        subprocess.call(args, shell=False)

    def move_completed_input(self):
        # Cleanup cover file
        if self.cover_path:
            os.remove(self.cover_path)
        # Move obsolete input to processed folder
        if Path(self.input_path.parent, 'done') == Path(config.junk_dir):
            logging.debug("Junk dir is direct parent")
            move_dir = Path(self.input_path)
        elif Path(self.input_path.parents[1], 'done') == Path(config.junk_dir):
            logging.debug("Junk dir is double parent")
            move_dir = Path(self.input_path.parent)
        else:
            logging.warning("Input path vs junk dir:")
            logging.warning(self.input_path)
            logging.warning(config.junk_dir)
            return logging.warning("Couldn't find junk dir relative to input")

        dest = Path(config.junk_dir, move_dir.name)
        try:
            move_dir.replace(dest)
        except OSError:
            logging.warning("Couldn't move input to complete dir")
