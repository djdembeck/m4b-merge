from datetime import datetime
import logging
import math
import os
import re
import requests
import shutil
import struct
import subprocess
from io import BytesIO
from pathlib import Path
from pathvalidate import sanitize_filename
from pydub.utils import mediainfo
# Local imports
from . import config, helpers
# Import mutagen for MP4 chapter manipulation
from mutagen.mp4 import MP4, MP4Chapters, Atom, Atoms


class M4bMerge:
    def __init__(self, input_data, metadata, original_path, chapters=None):
        self.input_path = input_data[0]
        self.input_extension = input_data[1]
        self.num_of_files = input_data[2]
        self.metadata = metadata
        self.original_path = original_path
        self.chapters = chapters

    def download_cover(self):
        if 'image' in self.metadata:
            # Request to image URL
            cover_request = requests.get(self.metadata['image'])
            # Verify image exists
            if cover_request.status_code == 200:
                # Path to write image to
                if self.input_path.is_dir():
                    self.cover_path = self.input_path / "cover.jpg"
                else:
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

        if 'authors' not in self.metadata:
            raise ValueError("No author in metadata")

        # Only use first author/narrator for file names;
        self.path_author = self.metadata['authors'][0]['name']

        # For embedded, use all authors/narrators
        author_name_arr = []
        for authors in self.metadata['authors']:
            author_name_arr.append(authors['name'])
        self.author = ', '.join(author_name_arr)

        narrator_name_arr = []
        if 'narrators' in self.metadata:
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

        if config.junk_dir:
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
            `author/series_name series_position - title`
            `: subtitle (year)/author - title (year)`
        """
        # First we need to replace the terms with actual data
        # ASIN
        self.replace_tag('asin', self.metadata['asin'])
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
        config.path_format = re.sub(
            tag, '', config.path_format
        ).rstrip().strip('-')

    def find_bitrate(self, file_input):
        # Divide bitrate by 1k, round up,
        # and return back to 1k divisible for round number.
        try:
            target_bitrate = math.ceil(
                int(mediainfo(file_input)['bit_rate']) / 1000
            ) * 1000
            logging.info(f"Source bitrate: {target_bitrate}")
        except KeyError:
            logging.warning("Unable to determine bitrate, using default")
            target_bitrate = ''

        return target_bitrate

    def find_samplerate(self, file_input):
        try:
            target_samplerate = int(
                mediainfo(
                    file_input
                )['sample_rate']
            )
            logging.info(f"Source samplerate: {target_samplerate}")
        except KeyError:
            logging.warning("Unable to determine samplerate, using default")
            target_samplerate = ''

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
                f" {self.title}")

    def merge_multiple_files(self):
        logging.info("Processing multiple files in a dir...")

        first_file = self.find_file_for_mediainfo()

        # Mediainfo data
        target_bitrate = self.find_bitrate(first_file)
        target_samplerate = self.find_samplerate(first_file)

        args = [
            config.m4b_tool_bin,
            'merge',
            f"--tmp-dir=/tmp/m4b-tool.{os.getpid()}",
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

        # Check if tone is available
        if config.tone_bin:
            logging.info("Using tone for metadata tagging...")

            # Build tone arguments
            tone_args = [config.tone_bin, 'tag', str(self.input_path)]

            # Map m4b-tool metadata args to tone format
            # m4b-tool format: --name=value, --album=value, etc.
            # tone format: --meta-title=value, --meta-album=value, etc.
            for arg in self.metadata_args:
                if arg.startswith('--name='):
                    tone_args.append(f'--meta-title={arg[7:]}')
                elif arg.startswith('--album='):
                    tone_args.append(f'--meta-album={arg[8:]}')
                elif arg.startswith('--artist='):
                    tone_args.append(f'--meta-artist={arg[9:]}')
                elif arg.startswith('--albumartist='):
                    tone_args.append(f'--meta-album-artist={arg[14:]}')
                elif arg.startswith('--year='):
                    tone_args.append(f'--meta-recording-date={arg[7:]}')
                elif arg.startswith('--description='):
                    tone_args.append(f'--meta-description={arg[14:]}')
                elif arg.startswith('--series='):
                    tone_args.append(f'--meta-movement-name={arg[9:]}')
                elif arg.startswith('--series-part='):
                    tone_args.append(f'--meta-part={arg[14:]}')
                elif arg.startswith('--genre='):
                    tone_args.append(f'--meta-genre={arg[8:]}')
                elif arg.startswith('--comment='):
                    tone_args.append(f'--meta-comment={arg[10:]}')
                elif arg.startswith('--cover='):
                    tone_args.append(f'--meta-cover-file={arg[8:]}')

            # Execute tone command
            logging.debug(f"Tone command: {tone_args}")
            subprocess.call(tone_args, shell=False)

            # Move completed file
            shutil.move(self.input_path, f"{self.book_output}.m4b")

        else:
            # Fall back to m4b-tool meta
            logging.info("Using m4b-tool for metadata tagging...")

            args = [
                config.m4b_tool_bin,
                'meta',
                f"--tmp-dir=/tmp/m4b-tool.{os.getpid()}",
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
            f"--tmp-dir=/tmp/m4b-tool.{os.getpid()}",
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
            selected_input_glob = Path(selected_input).glob('*')
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
            selected_input)
        logging.debug(
            (f"Guessed multi-disc extension to be:"
                f" {self.input_extension}")
        )
        return selected_input

    def _parse_timestamp(self, timestamp_str):
        """Parse a timestamp string (HH:MM:SS.mmm) to seconds."""
        parts = timestamp_str.split(':')
        if len(parts) == 3:
            hours, minutes, seconds = parts
            return float(hours) * 3600 + float(minutes) * 60 + float(seconds)
        return 0.0

    def _get_movie_timescale(self, fileobj, atoms):
        """Get the movie timescale from the mvhd atom."""
        try:
            mvhd_atom = atoms.path(b"moov", b"mvhd")[-1]
        except KeyError:
            return 1000

        chapters = MP4Chapters()
        chapters._parse_mvhd(mvhd_atom, fileobj)
        return chapters._timescale or 1000

    def _build_chpl_payload(self, chapters, timescale):
        """Build the chpl atom payload from chapter data."""
        if timescale <= 0:
            timescale = 1000

        body = bytearray()
        body.append(len(chapters))

        for seconds, title in chapters:
            safe_title = (title or "").strip()
            if not safe_title:
                safe_title = "Chapter"
            encoded = safe_title.encode("utf-8")[:255]
            start = int(round(seconds * timescale * 10000))
            body.extend(struct.pack(">Q", start))
            body.append(len(encoded))
            body.extend(encoded)

        header = struct.pack(">I", 0x01000000) + b"\x00\x00\x00\x00"
        return header + body

    def _apply_delta(self, helper, fileobj, parents, atoms, delta, offset):
        """Apply size delta to parent atoms."""
        if delta == 0:
            return
        from mutagen.mp4 import MP4Tags
        helper._MP4Tags__update_parents(fileobj, list(parents), delta)
        helper._MP4Tags__update_offsets(fileobj, atoms, delta, offset)

    def _replace_existing_chpl(self, helper, fileobj, atoms, chpl_atom, path):
        """Replace existing chpl atom with new one."""
        target = path[-1]
        offset = target.offset
        original_length = target.length
        from mutagen._util import resize_bytes
        resize_bytes(fileobj, original_length, len(chpl_atom), offset)
        fileobj.seek(offset)
        fileobj.write(chpl_atom)
        delta = len(chpl_atom) - original_length
        self._apply_delta(helper, fileobj, path[:-1], atoms, delta, offset)

    def _append_to_udta(self, helper, fileobj, atoms, chpl_atom, udta_path):
        """Append chpl atom to udta container."""
        from mutagen._util import insert_bytes
        udta_atom = udta_path[-1]
        insert_offset = udta_atom.offset + udta_atom.length
        insert_bytes(fileobj, len(chpl_atom), insert_offset)
        fileobj.seek(insert_offset)
        fileobj.write(chpl_atom)
        self._apply_delta(helper, fileobj, udta_path, atoms, len(chpl_atom), insert_offset)

    def _create_udta_with_chpl(self, helper, fileobj, atoms, chpl_atom, moov_path):
        """Create new udta container with chpl atom."""
        from mutagen._util import insert_bytes
        udta_atom = Atom.render(b"udta", chpl_atom)
        insert_offset = moov_path[-1].offset + moov_path[-1].length
        insert_bytes(fileobj, len(udta_atom), insert_offset)
        fileobj.seek(insert_offset)
        fileobj.write(udta_atom)
        self._apply_delta(helper, fileobj, moov_path, atoms, len(udta_atom), insert_offset)

    def _write_chapters_mutagen(self, m4b_path, chapter_markers):
        """Write chapter markers to MP4 file using mutagen."""
        if not chapter_markers:
            return

        seconds_markers = [(self._parse_timestamp(start_ms), title) for start_ms, title in chapter_markers]

        with open(m4b_path, "r+b") as fh:
            atoms = Atoms(fh)
            timescale = self._get_movie_timescale(fh, atoms)
            payload = self._build_chpl_payload(seconds_markers, timescale)
            chpl_atom = Atom.render(b"chpl", payload)
            from mutagen.mp4 import MP4Tags
            helper = MP4Tags()

            try:
                path = atoms.path(b"moov", b"udta", b"chpl")
            except KeyError:
                try:
                    udta_path = atoms.path(b"moov", b"udta")
                except KeyError:
                    moov_path = atoms.path(b"moov")
                    self._create_udta_with_chpl(helper, fh, atoms, chpl_atom, moov_path)
                else:
                    self._append_to_udta(helper, fh, atoms, chpl_atom, udta_path)
            else:
                self._replace_existing_chpl(helper, fh, atoms, chpl_atom, path)

        # Verify chapters were written correctly
        try:
            mp4 = MP4(m4b_path)
            _ = mp4.chapters
        except Exception as e:
            logging.warning(f"Failed to verify chapters: {e}")

    def _parse_chapter_file(self, chapter_file):
        """Parse chapter file and return list of (timestamp, title) tuples."""
        chapters = []
        with open(chapter_file, 'r') as f:
            for line in f:
                line = line.strip()
                # Skip comments and empty lines
                if not line or line.startswith('#'):
                    continue
                # Parse timestamp and title
                parts = line.split(None, 1)
                if len(parts) >= 2:
                    timestamp = parts[0]
                    title = parts[1]
                    chapters.append((timestamp, title))
        return chapters

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

        # Check if mp4chaps is available
        if config.mp4chaps_bin:
            logging.info("Using mp4chaps for chapter operations")
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
        else:
            logging.info("Using mutagen for chapter operations (mp4chaps not available)")
            # Parse chapter file
            chapter_markers = self._parse_chapter_file(chapter_file)
            if not chapter_markers:
                logging.error("Chapter file is empty, attempting to correct")
                # Try to repair by using existing chapters from the file
                try:
                    mp4 = MP4(m4b_to_modify)
                    if mp4.chapters:
                        chapter_markers = [(str(chapter.start), chapter.title) for chapter in mp4.chapters]
                except Exception as e:
                    logging.warning(f"Could not repair chapters: {e}")
            # Write chapters using mutagen
            self._write_chapters_mutagen(m4b_to_modify, chapter_markers)

    def move_completed_input(self):
        if not config.junk_dir:
            return
        # Cleanup cover file
        if self.cover_path:
            os.remove(self.cover_path)
        # Move completed input to junk dir
        logging.debug(
            f'Moving completed input {self.original_path} to {config.junk_dir}')
        dest = Path(config.junk_dir, self.original_path.name)

        try:
            shutil.move(self.original_path, dest)
        except OSError:
            logging.warning("Couldn't move input to complete dir")
