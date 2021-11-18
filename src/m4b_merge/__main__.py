from pathlib import Path
import argparse
import logging
import os
# Local imports
from . import audible_helper, config, helpers, m4b_helper


def run_all(inputs):
    print('-' * 50)
    print(f"Working on: {inputs}")
    print('-' * 50)
    # Validate path, check if it's a directory or a file
    # This will also run find_extension to determine relevant filetype
    input_data = helpers.get_directory(inputs)

    # Validate ASIN input
    while True:
        try:
            asin = input("Audiobook ASIN: ")
            helpers.validate_asin(config.api_url, asin)
            break
        except Exception as e:
            print(str(e))

    # Create BookData object from asin response
    aud = audible_helper.BookData(asin)
    metadata = aud.fetch_api_data(config.api_url)
    chapters = aud.get_chapters()

    # Process metadata and run components to merge files
    m4b = m4b_helper.M4bMerge(input_data, metadata, chapters)
    m4b.run_merge()


def validate_args(args):
    # API URL
    if args.api_url:
        config.api_url = args.api_url
    else:
        config.api_url = "https://api.audnex.us"
    # Completed Directory
    if args.completed_directory:
        config.junk_dir = args.completed_directory
    else:
        # If using docker, default to /input/done folder, else $USER/input/done
        if Path('/input').is_dir():
            config.junk_dir = Path('/input/done')
        else:
            default_input = Path.home()
            config.junk_dir = Path(f"{default_input}/input/done")
    # Log Level
    if args.log_level:
        numeric_level = getattr(logging, args.log_level.upper(), None)
        if not isinstance(numeric_level, int):
            raise ValueError('Invalid log level: %s' % args.log_level)
        logging.basicConfig(level=numeric_level)
    else:
        logging.basicConfig(level=os.environ.get("LOG_LEVEL", "INFO"))
    # Number of CPUs
    if args.num_cpus:
        config.num_cpus = args.num_cpus
    else:
        config.num_cpus = os.cpu_count()
    # Output Directory
    if args.output:
        config.output = args.output
    else:
        # If using docker, default to /output folder, else $USER/output
        if Path('/output').is_dir():
            config.output = Path('/output')
        else:
            default_output = Path.home()
            config.output = Path(f"{default_output}/output")
    # Path Format
    if args.path_format:
        config.path_format = args.path_format
    else:
        config.path_format = "author/title - subtitle"
    # Inputs
    # Last to be checked
    if args.inputs:
        for inputs in args.inputs:
            if inputs.exists():
                run_all(inputs)
            else:
                logging.error(f"Input \"{inputs}\" does not exist")

        print('-' * 25)
        print(f"Done processing {len(args.inputs)} inputs")
        print('-' * 25)


def main():
    parser = argparse.ArgumentParser(
        description='m4bmerge cli'
    )
    parser.add_argument(
        "--api_url",
        help="Audnexus mirror to use",
        type=str
    )
    parser.add_argument(
        "--completed_directory",
        help="Directory path to move original input files to",
        type=Path
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
    parser.add_argument(
        "--num_cpus",
        help="Number of CPUs to use",
        type=int
    )
    parser.add_argument(
        "-o", "--output",
        help="Output directory",
        type=Path
    )
    parser.add_argument(
        "-p", "--path_format",
        help="Structure of output path/naming. Supported terms: author, narrator, series_name, series_position, subtitle, title, year",
        type=str
    )

    validate_args(parser.parse_args())

# Only run call if using CLI directly
if __name__ == "__main__":
    main()
