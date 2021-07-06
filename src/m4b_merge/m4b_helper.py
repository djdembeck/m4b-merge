import logging, math, os, shutil
from pathlib import Path
from pathvalidate import sanitize_filename
from pydub.utils import mediainfo
# Local imports
from . import config

class M4bMerge:
	def __init__(self, input_data, metadata):
		self.input_path = input_data[0]
		self.input_extension = input_data[1]
		self.num_of_files = input_data[2]
		self.metadata = metadata

	def prepare_data(self):
		## Metadata variables
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
		path_narrator = self.metadata['narrators'][0]
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
		##

		## Make necessary directories
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

		## Array for argument use
		# main metadata args
		self.metadata_args = [
			f"--name=\"{title}\"",
			f"--album=\"{path_title}\"",
			f"--artist=\"{narrator}\"",
			f"--albumartist=\"{author}\"",
			f"--year=\"{year}\"",
			f"--description=\"{summary}\""
		]

		# Append series to metadata if it exists
		if series:
			self.metadata_args.append(f"--series=\"{series}\"")

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

		# args for multiple input files in a folder
		if (self.input_path.is_dir() and self.num_of_files > 1) or self.input_extension == None:
			logging.info("Processing multiple files in a dir...")

			# If multi-disc, find the extension
			if not self.input_extension:
				input_path_glob = Path(self.input_path).glob('**/*')
				i = 0
				sorted_multi=sorted(input_path_glob)
				while not sorted_multi[i].is_dir():
					logging.debug("Looking for first dir in multi-dir...")
					i += 1
				selected_input = sorted_multi[i]
				logging.debug(f"Result was #{i+1} for first dir: {selected_input}")
				self.input_extension = find_extension(selected_input)[1]
				logging.debug(f"Guessed multi-disc extension to be: {self.input_extension}")
			else:
				selected_input = self.input_path

			selected_input_glob = Path(selected_input).glob('**/*')

			# Find first file with our extension, to check rates against
			for file in sorted(selected_input_glob):
				if file.suffix == f".{self.input_extension}":
					first_file = file
					break

			logging.debug(f"Got file to run mediainfo on: {first_file}")

			## Mediainfo data
			target_bitrate = self.find_bitrate(first_file)
			target_samplerate = self.find_samplerate(first_file)
			##

			args = [
				' merge',
				f"--output-file=\"{self.book_output}/{self.file_title}.m4b\""
			]

			# Add in main metadata and merge args
			args.extend(self.metadata_args)
			args.extend(self.processing_args)

			if self.input_extension == "m4b" or self.input_extension == "m4a":
				logging.info(f"Multiple {self.input_extension} files, not converting")
				args.append(f'--no-conversion')
			else:
				args.append(
					f"--audio-bitrate=\"{target_bitrate}\"",
					f"--audio-samplerate=\"{target_samplerate}\""
				)

			# m4b command with passed args
			m4b_cmd = (
				config.m4b_tool_bin + 
			' '.join(args) + 
			f" \"{self.input_path}\""
			)
			logging.debug(f"M4B command: {m4b_cmd}")
			os.system(m4b_cmd)

			# Move obsolete input to processed folder
			self.move_completed_input()

			self.fix_chapters()
			
		# args for single m4b input file
		elif (self.input_path.is_file()) and (self.input_extension == "m4b" or self.input_extension == "m4a"):
			logging.info(f"Processing single {self.input_extension} input...")

			m4b_cmd = (
				config.m4b_tool_bin + 
			' meta ' + 
			f'--export-chapters=\"\"' + 
			f" \"{self.input_path}\""
			)
			logging.debug(f"M4B command: {m4b_cmd}")
			os.system(m4b_cmd)
			
			shutil.move(
				f"{self.input_path.parent}/{self.input_path.stem}.chapters.txt",
				f"{self.book_output}/{self.file_title}.chapters.txt"
				)

			args = [
				' meta'
			]
			# Add in main metadata args
			args.extend(self.metadata_args)

			# make backup file
			shutil.copy(
				self.input_path,
				f"{self.input_path.parent}/{self.input_path.stem}.new.m4b"
				)

			# m4b command with passed args
			m4b_cmd = (
				config.m4b_tool_bin + 
				' '.join(args) + 
				f" \"{self.input_path.parent}/{self.input_path.stem}.new.m4b\"")
			logging.debug(f"M4B command: {m4b_cmd}")
			os.system(m4b_cmd)

			# Move completed file
			shutil.move(
				f"{self.input_path.parent}/{self.input_path.stem}.new.m4b",
				f"{self.book_output}/{self.file_title}.m4b"
			)

			self.move_completed_input()

			self.fix_chapters()

		elif self.input_path.is_file() and self.input_extension == "mp3":
			logging.info(f"Processing single {self.input_extension} input...")

			## Mediainfo data
			target_bitrate = self.find_bitrate(self.input_path)
			target_samplerate = self.find_samplerate(self.input_path)
			##

			args = [
				' merge',
				f"--output-file=\"{self.book_output}/{self.file_title}.m4b\"",
				f"--audio-bitrate=\"{target_bitrate}\"",
				f"--audio-samplerate=\"{target_samplerate}\"",
				'--skip-cover'
			]
			# Add in main metadata and merge args
			args.extend(self.metadata_args)
			args.extend(self.processing_args)

			# m4b command with passed args
			m4b_cmd = (
				config.m4b_tool_bin + 
			' '.join(args) + 
			f" \"{self.input_path}\""
			)
			logging.debug(f"M4B command: {m4b_cmd}")
			os.system(m4b_cmd)

			self.move_completed_input()

			logging.warning(f"Not processing chapters for  {title}, since it's an mp3")

		elif not self.input_extension:
			logging.error(f"No recognized filetypes found for {title}")

		else:
			logging.error(f"Couldn't determine input type/extension for {title}")

	def fix_chapters(self):
		chapter_file = f"{self.book_output}/{self.file_title}.chapters.txt"
		m4b_to_modify = f"{self.book_output}/{self.file_title}.m4b"
		new_file_content = ""
		with open(chapter_file) as f:
			# Store and then skip past total length section
			for line in f:
				if "# total-length" in line.strip():
					new_file_content += line.strip() +"\n"
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
				new_file_content += new_line +"\n"

		with open(chapter_file, 'w') as f:
			f.write(new_file_content)
		
		# Apply fixed chapters to file
		m4b_chap_cmd = (
			config.m4b_tool_bin + 
			' meta ' + 
			f" \"{m4b_to_modify}\" " + 
			f"--import-chapters=\"{chapter_file}\""
			)
		os.system(m4b_chap_cmd)

	def move_completed_input(self):
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