from pathlib import Path
import argparse, collections, logging, math, os, shutil
from pathvalidate import sanitize_filename
from pydub.utils import mediainfo
# Local imports
import audiblehelper, config, helpers

def find_extension(dirpath):
	EXTENSIONS=['mp3', 'm4a', 'm4b']

	for EXT in EXTENSIONS:
		if collections.Counter(
			p.suffix for p in Path(dirpath)
				.resolve().glob(f'*.{EXT}')
			):
			USE_EXT = EXT
			list_of_files = os.listdir(Path(dirpath))
			# Case for single file in a folder
			if sum(
				x.endswith(f'.{USE_EXT}') 
				for x in list_of_files
				) == 1:
				for m4b_file in Path(dirpath).glob(f'*.{USE_EXT}'):
					logging.debug(f"Adjusted input for {dirpath} to use single m4b file")
					dirpath = m4b_file
				num_of_files = 1
			else:
				num_of_files = sum(
					x.endswith(f'.{USE_EXT}') 
					for x in list_of_files
					)
			return dirpath, USE_EXT, num_of_files

def get_directory(input_take):
	# Check if input is a dir
	if Path(input_take).is_dir():
		# Check if input has multiple subdirs
		num_of_subdirs = len(next(os.walk(input_take))[1])
		if num_of_subdirs >= 1:
			logging.info(
				f"Found multiple ({num_of_subdirs}) subdirs, "
					f"using those as input (multi-disc)"
				)
			dirpath = input_take
			USE_EXT = None
			num_of_files = num_of_subdirs
		else:
			for dirpath, dirnames, files in os.walk(input_take):
				find_ext = find_extension(dirpath)
				dirpath = find_ext[0]
				USE_EXT = find_ext[1]
				num_of_files = find_ext[2]

	# Check if input is a file
	elif Path(input_take).is_file():
		dirpath = input_take
		USE_EXT_PRE = dirpath.suffix
		USE_EXT = Path(USE_EXT_PRE).stem.split('.')[1]
		num_of_files = 1

	logging.debug(f"Final input path is: {dirpath}")
	logging.debug(f"Extension is: {USE_EXT}")
	logging.debug(f"Number of files: {num_of_files}")
	return Path(dirpath), USE_EXT, num_of_files

def m4b_data(input_data, metadata):
	## Checks
	# Find path to m4b-tool binary
	m4b_tool = shutil.which(config.m4bpath)

	# Check that binary actually exists
	if not m4b_tool:
		# try to automatically recover
		if shutil.which('m4b-tool'):
			m4b_tool = shutil.which('m4b-tool')
		else:
			raise SystemExit(
				'Error: Cannot find m4b-tool binary.'
				)
	# If no response from binary, exit
	if not m4b_tool:
		raise SystemExit(
			'Error: Could not successfully run m4b-tool, exiting.'
			)

	## Metadata variables
	# Only use subtitle in case of metadata, not file name
	if 'subtitle' in metadata:
		base_title = metadata['title']
		base_subtitle = metadata['subtitle']
		title = f"{base_title} - {base_subtitle}"
	else:
		title = metadata['title']
	# Only use first author/narrator for file names;
	# no subtitle for file name
	path_title = metadata['title']
	path_author = metadata['authors'][0]['name']
	path_narrator = metadata['narrators'][0]
	# For embedded, use all authors/narrators
	author_name_arr = []
	for authors in metadata['authors']:
		author_name_arr.append(authors['name'])
	author = ', '.join(author_name_arr)
	narrator = ', '.join(metadata['narrators'])
	if 'series' in metadata:
		series = metadata['series']
	else:
		series = None
	summary = metadata['short_summary']
	year = metadata['release_date'].year

	book_output = (
		f"{config.output}/{sanitize_filename(path_author)}/"
		f"{sanitize_filename(path_title)}"
	)
	file_title = sanitize_filename(title)
	##

	## File variables
	in_dir = input_data[0]
	in_ext = input_data[1]
	num_of_files = input_data[2]
	##

	## Make necessary directories
	# Final output folder
	Path(book_output).mkdir(
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
	metadata_args = [
		f"--name=\"{title}\"",
		f"--album=\"{path_title}\"",
		f"--artist=\"{narrator}\"",
		f"--albumartist=\"{author}\"",
		f"--year=\"{year}\"",
		f"--description=\"{summary}\""
	]

	# args for merge  process
	processing_args = [
		'--force',
		'--no-chapter-reindexing',
		'--no-cleanup',
		f'--jobs={config.num_cpus}'
	]

	# Set logging level of m4b-tool depending upon log_level
	level = logging.root.level
	if level == logging.INFO:
		processing_args.append('-v')
	elif level == logging.DEBUG:
		processing_args.append('-vvv')

	# args for multiple input files in a folder
	if (in_dir.is_dir() and num_of_files > 1) or in_ext == None:
		logging.info("Processing multiple files in a dir...")

		in_dir_glob = Path(in_dir).glob('**/*')

		# If multi-disc, find the extension
		if not in_ext:
			i = 0
			sorted_multi=sorted(in_dir_glob)
			while not sorted_multi[i].is_dir():
				logging.debug("Looking for first dir in multi-dir...")
				i += 1
			dir_select = sorted_multi[i]
			logging.debug(f"Result was #{i+1} for first dir: {dir_select}")
			in_ext = find_extension(dir_select)[1]
			logging.debug(f"Guessed multi-disc extension to be: {in_ext}")
		else:
			dir_select = in_dir

		dir_select_glob = Path(dir_select).glob('**/*')

		# Find first file with our extension, to check rates against
		for file in sorted(dir_select_glob):
			if file.suffix == f".{in_ext}":
				first_file = file
				break

		logging.debug(f"Got file to run mediainfo on: {first_file}")

		## Mediainfo data
		# Divide bitrate by 1k, round up,
		# and return back to 1k divisible for round number.
		target_bitrate = math.ceil(
			int(mediainfo(first_file)['bit_rate']) / 1000
		) * 1000

		target_samplerate = int(
			mediainfo(
				first_file
			)['sample_rate']
		)

		logging.info(f"Source bitrate: {target_bitrate}")
		logging.info(f"Source samplerate: {target_samplerate}")
		##
		args = [
			' merge',
			f"--output-file=\"{book_output}/{file_title}.m4b\"",
			f"--audio-bitrate=\"{target_bitrate}\"",
			f"--audio-samplerate=\"{target_samplerate}\""
		]
		# Add in main metadata and merge args
		args.extend(metadata_args)
		args.extend(processing_args)

		if series:
			args.append(f"--series=\"{series}\"")

		if in_ext == "m4b" or in_ext == "m4a":
			logging.info(f"Multiple {in_ext} files, not converting")
			args.append(f'--no-conversion')

		# m4b command with passed args
		m4b_cmd = (
			m4b_tool + 
		' '.join(args) + 
		f" \"{in_dir}\""
		)
		logging.debug(f"M4B command: {m4b_cmd}")
		os.system(m4b_cmd)

		# Move obsolete input to processed folder
		shutil.move(
			f"{in_dir}",
			f"{config.junk_dir}"
		)

		m4b_fix_chapters(
			f"{book_output}/{file_title}.chapters.txt",
			f"{book_output}/{file_title}.m4b",
			m4b_tool
			)
		
	# args for single m4b input file
	elif (in_dir.is_file()) and (in_ext == "m4b" or in_ext == "m4a"):
		logging.info(f"Processing single {in_ext} input...")

		## Mediainfo data
		# Divide bitrate by 1k, round up,
		# and return back to 1k divisible for round number.
		target_bitrate = math.ceil(
			int(mediainfo(in_dir)['bit_rate']) / 1000
		) * 1000

		target_samplerate =	int(mediainfo(in_dir)['sample_rate'])
		##

		m4b_cmd = (
			m4b_tool + 
		' meta ' + 
		f'--export-chapters=\"\"' + 
		f" \"{in_dir}\""
		)
		logging.debug(f"M4B command: {m4b_cmd}")
		os.system(m4b_cmd)
		
		shutil.move(
			f"{in_dir.parent}/{in_dir.stem}.chapters.txt",
			f"{book_output}/{file_title}.chapters.txt"
			)

		args = [
			' meta'
		]
		# Add in main metadata args
		args.extend(metadata_args)

		if series:
			args.append(f"--series=\"{series}\"")

		# make backup file
		shutil.copy(
			in_dir,
			f"{in_dir.parent}/{in_dir.stem}.new.m4b"
			)

		# m4b command with passed args
		m4b_cmd = (
			m4b_tool + 
			' '.join(args) + 
			f" \"{in_dir.parent}/{in_dir.stem}.new.m4b\"")
		logging.debug(f"M4B command: {m4b_cmd}")
		os.system(m4b_cmd)

		# Move completed file
		shutil.move(
			f"{in_dir.parent}/{in_dir.stem}.new.m4b",
			f"{book_output}/{file_title}.m4b"
		)

		# Move obsolete input to processed folder
		if Path(in_dir.parent, 'done') == config.junk_dir:
			logging.debug("Junk dir is direct parent")
			move_dir = in_dir
		elif Path(in_dir.parents[1], 'done') == config.junk_dir:
			logging.debug("Junk dir is double parent")
			move_dir = in_dir.parent
		else:
			logging.warning("Couldn't find junk dir relative to input")

		if move_dir:
			shutil.move(
				f"{move_dir}",
				f"{config.junk_dir}"
			)

		m4b_fix_chapters(
			f"{book_output}/{file_title}.chapters.txt",
			f"{book_output}/{file_title}.m4b",
			m4b_tool
			)

	elif in_dir.is_file() and in_ext == "mp3":
		logging.info(f"Processing single {in_ext} input...")

		## Mediainfo data
		# Divide bitrate by 1k, round up,
		# and return back to 1k divisible for round number.
		target_bitrate = math.ceil(
			int(mediainfo(f"{in_dir}")['bit_rate']) / 1000
		) * 1000

		target_samplerate = int(
			mediainfo(
				f"{in_dir}"
			)['sample_rate']
		)

		logging.info(f"Source bitrate: {target_bitrate}")
		logging.info(f"Source samplerate: {target_samplerate}")
		##
		args = [
			' merge',
			f"--output-file=\"{book_output}/{file_title}.m4b\"",
			f"--audio-bitrate=\"{target_bitrate}\"",
			f"--audio-samplerate=\"{target_samplerate}\"",
			'--skip-cover'
		]
		# Add in main metadata and merge args
		args.extend(metadata_args)
		args.extend(processing_args)

		if series:
			args.append(f"--series=\"{series}\"")

		# m4b command with passed args
		m4b_cmd = (
			m4b_tool + 
		' '.join(args) + 
		f" \"{in_dir}\""
		)
		logging.debug(f"M4B command: {m4b_cmd}")
		os.system(m4b_cmd)

		# Move obsolete input to processed folder
		if Path(in_dir.parent, 'done') == config.junk_dir:
			logging.debug("Junk dir is direct parent")
			move_dir = in_dir
		elif Path(in_dir.parents[1], 'done') == config.junk_dir:
			logging.debug("Junk dir is double parent")
			move_dir = in_dir.parent
		else:
			logging.warning("Couldn't find junk dir relative to input")

		if move_dir:
			shutil.move(
				f"{move_dir}",
				f"{config.junk_dir}"
			)

		logging.warning(f"Not processing chapters for  {title}, since it's an mp3")

	elif not in_ext:
		logging.error(f"No recognized filetypes found for {title}")

	else:
		logging.error(f"Couldn't determine input type/extension for {title}")

def m4b_fix_chapters(input, target, m4b_tool):
	new_file_content = ""
	with open(input) as f:
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

	with open(input, 'w') as f:
		f.write(new_file_content)
	
	# Apply fixed chapters to file
	m4b_chap_cmd = (
		m4b_tool + 
		' meta ' + 
		f" \"{target}\" " + 
		f"--import-chapters=\"{input}\""
		)
	os.system(m4b_chap_cmd)

def main(inputs):
	logging.info(f"Working on: {inputs}")
	input_data = get_directory(inputs)
	asin = input("Audiobook ASIN: ")
	aud = audiblehelper.AudibleData(asin)
	metadata = aud.parser()
	m4b_data(input_data, metadata)

# Only run call if using CLI directly
if __name__ == "__main__":
	# Setup global variables

	parser = argparse.ArgumentParser(
		description='Bragi Books merge cli'
		)
	parser.add_argument(
		"-i", "--inputs",
		help="Input paths to process",
		nargs='+',
		required=True,
		type=Path
		)
	parser.add_argument(
		"--log_level",
		help="Set logging level"
		)
	args = parser.parse_args()

	# Get log level from system or input
	if args.log_level:
		numeric_level = getattr(logging, args.log_level.upper(), None)
		if not isinstance(numeric_level, int):
			raise ValueError('Invalid log level: %s' % args.log_level)
		logging.basicConfig(level=numeric_level)
	else:
		logging.basicConfig(level=os.environ.get("LOG_LEVEL", "INFO"))
	# Run through inputs
	for inputs in args.inputs:
		if inputs.exists():
			main(inputs)
		else:
			logging.error(f"Input \"{inputs}\" does not exist")