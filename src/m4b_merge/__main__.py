from pathlib import Path
import argparse
import logging
import os
# Local imports
from . import audible_helper, helpers, m4b_helper


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
            helpers.validate_asin(asin)
            break
        except Exception as e:
            print(str(e))

    # Create BookData object from asin response
    aud = audible_helper.BookData(asin)
    metadata = aud.parser()
    chapters = aud.get_chapters()

    # Process metadata and run components to merge files
    m4b = m4b_helper.M4bMerge(input_data, metadata, chapters)
    m4b.run_merge()


def main():
    parser = argparse.ArgumentParser(
        description='m4bmerge cli'
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
            run_all(inputs)
        else:
            logging.error(f"Input \"{inputs}\" does not exist")

    print('-' * 25)
    print(f"Done processing {len(args.inputs)} inputs")
    print('-' * 25)


# Only run call if using CLI directly
if __name__ == "__main__":
    main()
