[![PyPI](https://img.shields.io/pypi/v/m4b-merge)](https://pypi.org/project/m4b-merge/)
[![GitHub](https://img.shields.io/github/license/djdembeck/m4b-merge)](https://github.com/djdembeck/m4b-merge/blob/develop/LICENSE)
![PyPI - Python Version](https://img.shields.io/pypi/pyversions/m4b-merge?style=flat)
[![Python package](https://github.com/djdembeck/m4b-merge/actions/workflows/build.yml/badge.svg)](https://github.com/djdembeck/m4b-merge/actions/workflows/build.yml)
[![CodeFactor Grade](https://img.shields.io/codefactor/grade/github/djdembeck/m4b-merge)](https://www.codefactor.io/repository/github/djdembeck/m4b-merge)
[![PyPI - Downloads](https://img.shields.io/pypi/dm/m4b-merge)](https://pypi.org/project/m4b-merge/)
## Functionality
The steps accomplished by using this tool are as follows:

- Easy batch inputs via `-i folder1/ folder2/* file.mp3` etc.
- Only user input required is one ASIN per book.
- Converts mp3(s) into single m4b files
  - Matches existing bitrate and samplerate for target file.
  - Standardizes chapter names, like 'Chapter 1'. 
- Merges or edits m4a/m4b into single m4b file, without re-converting.
- Moves input file/folder to `done` folder when processed.

Audible data features:
  - Title, author(s), narrator(s), series, release year, description
    - For generating folder structure 
    - For seeding M4b metadata fields
  - Chapters (title and length) used for m4b/m4a inputs.


## Dependencies
This tool uses m4b-merge for it's file processing. Installation instructions can be found on [the project's readme](https://github.com/sandreas/m4b-tool#installation).

## CLI usage

```
usage: m4b-merge [-h] -i INPUTS [INPUTS ...] [--log_level LOG_LEVEL]

m4b-merge cli

optional arguments:
  -h, --help            show this help message and exit
  -i INPUTS [INPUTS ...], --inputs INPUTS [INPUTS ...]
                        Input paths to process
  --log_level LOG_LEVEL
                        Set logging level
```
  - Check the user editable variables in [config.py](src/m4b_merge/config.py), and see if there's anything you need to change.
  - On first run, you will be prompted to signin to Audible. This is a one-time process that will be saved to your system's relevant config folder, under `m4b-merge`.

## Module usage
If you are a developer wanting to use this in a project, you can import the modules as so:
`from m4b_merge import audible_helper, config, helpers, m4b_helper`

And then creating the objects you need (from `audible_helper.BookData(asin)` and `m4b_helper.M4bMerge(input_data, metadata)`)
You can see more usage examples in the sister project, [Bragi Books](https://github.com/djdembeck/bragibooks/blob/main/importer/views.py)

The `parser` function in `audible_helper.BookData` returns some extra data not used in the CLI here. This is a list of all data returned:
- Title
- Short Summary
- Long Summary
- Authors
- Narrators
- Series
- Release Date
- Publisher
- Language
- Runtime in minutes
- Format type (abridged, unabridged, other)

## Credits
- Many thanks to mkb79 for their [audible](https://github.com/mkb79/Audible) package.
- Thanks to sandreas for their tireless work on [m4b-tool](https://github.com/sandreas/m4b-tool)