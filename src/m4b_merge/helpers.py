from pathlib import Path
import collections
import logging
import os
import requests


# Find the primary extension which we will use
def find_extension(path_to_check):
    EXTENSIONS = ['mp3', 'm4a', 'm4b']

    for EXT in EXTENSIONS:
        if collections.Counter(
            p.suffix for p in Path(path_to_check)
                .resolve().glob(f'*.{EXT}')
                ):
            extension_to_use = EXT
            list_of_files = os.listdir(Path(path_to_check))
            # Case for single file in a folder
            if sum(
                x.endswith(f'.{extension_to_use}')
                for x in list_of_files
            ) == 1:
                for m4b_file in (
                    Path(path_to_check)
                        .glob(f'*.{extension_to_use}')):
                    logging.debug(
                        f"Adjusted input for {path_to_check}"
                        f" to use single m4b file")
                    path_to_use = m4b_file
                num_of_files = 1
            else:
                num_of_files = sum(
                    x.endswith(f'.{extension_to_use}')
                    for x in list_of_files
                    )
                path_to_use = path_to_check
            return path_to_use, extension_to_use, num_of_files


# Validate path, check if it's a directory or a file
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
            path_to_use = input_take
            extension_to_use = None
            num_of_files = num_of_subdirs
        else:
            for dirpath, dirnames, files in os.walk(input_take):
                return_find_ext = find_extension(dirpath)
                path_to_use = return_find_ext[0]
                extension_to_use = return_find_ext[1]
                num_of_files = return_find_ext[2]

    # Check if input is a file
    elif Path(input_take).is_file():
        path_to_use = input_take
        extension_to_use_PRE = path_to_use.suffix
        extension_to_use = Path(extension_to_use_PRE).stem.split('.')[1]
        num_of_files = 1

    logging.debug(f"Final input path is: {path_to_use}")
    logging.debug(f"Extension is: {extension_to_use}")
    logging.debug(f"Number of files: {num_of_files}")
    return Path(path_to_use), extension_to_use, num_of_files


# Checks that asin is the expected length, then cheks for http code 200
def validate_asin(asin):
    if len(asin) == 10:
        # Check that asin actually returns data from audible
        check = requests.get(f"https://www.audible.com/pd/{asin}")
        if check.status_code == 200:
            logging.info(f"Validated ASIN: {asin}")
        else:
            raise ValueError(f"HTTP error {check.status_code}")

        return check.status_code

    raise ValueError("Invalid ASIN length")
