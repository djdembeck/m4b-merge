
<h1 align="center">m4b-merge</h1>

<div align="center">

[![Status](https://img.shields.io/badge/status-active-success.svg)]()
[![GitHub Issues](https://img.shields.io/github/issues/djdembeck/m4b-merge.svg)](https://github.com/djdembeck/m4b-merge/issues)
[![GitHub Pull Requests](https://img.shields.io/github/issues-pr/djdembeck/m4b-merge.svg)](https://github.com/djdembeck/m4b-merge/pulls)
[![License](https://img.shields.io/github/license/djdembeck/m4b-merge)](https://github.com/djdembeck/m4b-merge/blob/develop/LICENSE)
[![PyPI](https://img.shields.io/pypi/v/m4b-merge)](https://pypi.org/project/m4b-merge/)
![PyPI - Python Version](https://img.shields.io/pypi/pyversions/m4b-merge?style=flat)
[![Python package](https://github.com/djdembeck/m4b-merge/actions/workflows/build.yml/badge.svg)](https://github.com/djdembeck/m4b-merge/actions/workflows/build.yml)
[![CodeFactor Grade](https://img.shields.io/codefactor/grade/github/djdembeck/m4b-merge)](https://www.codefactor.io/repository/github/djdembeck/m4b-merge)
[![PyPI - Downloads](https://img.shields.io/pypi/dm/m4b-merge)](https://pypi.org/project/m4b-merge/)

</div>

---

<p align="center"> A CLI tool which outputs consistently sorted, tagged, single m4b files regardless of the input.
    <br> 
</p>

## 📝 Table of Contents

- [About](#about)
- [Getting Started](#getting_started)
- [Usage](#usage)
- [Built Using](#built_using)
- [Contributing](../CONTRIBUTING.md)
- [Authors](#authors)
- [Acknowledgments](#acknowledgement)

## 🧐 About <a name = "about"></a>

m4b-merge was originally part of [Bragi Books](https://github.com/djdembeck/bragibooks), but was split apart to allow savvy users to automate its usage in more advanced ways. Some of the things m4b-merge offers are:

- Accepts single and multiple mp3, m4a and m4b files.
- mp3s are converted to m4b. m4a/m4b files are edited/merged without conversion.
- Matches existing bitrate and samplerate for target file conversions.
- Final files moved to `/output/Author/Book/Book: Subtitle.m4b` format.
- Moves finished files into `done` folder in `input` directory.

Metadata provided by [audnexus](https://github.com/laxamentumtech/audnexus):

- Title, authors, narrators, description, series, genres, release year - written as tags.
- Chapter times/titles (only when input is m4b or a single mp3) - written as tags and `chapters.txt`.
- High resolution (2000x2000 or greater) cover art - embedded into output file.

## 🏁 Getting Started <a name = "getting_started"></a>

### Prerequisites

You can either install this project via `pip` directly or run it prepackaged in Docker:
- If installing directly on your system, you'll need to install m4b-tool and it's dependants from [the project's readme](https://github.com/sandreas/m4b-tool#installation)
- If using Docker, all prerequisites are included in the image.

### Installing

#### For a `pip` installation

Simply run

```
pip install m4b-merge
```

#### For a Docker installation

You'll need to specify input/output volumes in the run command for easy use later:

```
docker run --name=merge -v /path/to/input:/input -v /path/to/output:/output ghcr.io/djdembeck/m4b-merge:main
```

You may also specify the user and group to run as with env variables:

```
-e UID=99 -e GID=100
```

## 🔧 Running the tests <a name = "tests"></a>

- Run `pip install pytest`
- To run all tests, run `pytest` from inside this project directory.
- To run a single test, run `pytest tests/test_NAME.py`

## 🎈 Usage <a name="usage"></a>

### Workflow
The process is simple
1. Pass the file as input via `-i FILE.ext` or folder `-i DIR/`
2. Enter the ASIN (found on audible.com) when prompted.
3. Depending on necessary conversions, the process will take between 5 seconds and 5-10 minutes.

### CLI usage
```
usage: m4b-merge [-h] [--api_url API_URL] [--completed_directory COMPLETED_DIRECTORY] -i INPUTS [INPUTS ...] [--log_level LOG_LEVEL]
                 [--num_cpus NUM_CPUS] [-o OUTPUT]

m4bmerge cli

optional arguments:
  -h, --help            show this help message and exit
  --api_url API_URL     Audnexus mirror to use
  --completed_directory COMPLETED_DIRECTORY
                        Directory path to move original input files to
  -i INPUTS [INPUTS ...], --inputs INPUTS [INPUTS ...]
                        Input paths to process
  --log_level LOG_LEVEL
                        Set logging level
  --num_cpus NUM_CPUS   Number of CPUs to use
  -o OUTPUT, --output OUTPUT
                        Output directory
  -p PATH_FORMAT, --path_format PATH_FORMAT
                        Structure of output path/naming.Supported terms: author, narrator, series_name, series_position, subtitle, title, year

```

#### When installed via `pip`, you can run inputs like so

```
m4b-merge -i /path/to/file.mp3
```

Or for multiple inputs

```
m4b-merge -i /path/to/file.mp3 /dir/ /path/to/other/file
```

#### On Docker, you can run inputs like so

```
docker run -it merge m4b-merge -i /input/file.mp3
```

For a folder of multiple audio files, simply pass the folder itself as an input, such as `-i /input/dir`

## ⛏️ Built Using <a name = "built_using"></a>

- [audnexus](https://github.com/laxamentumtech/audnexus) - API backend for metadata
- [m4b-tool](https://github.com/sandreas/m4b-tool) - File merging and tagging

## ✍️ Authors <a name = "authors"></a>

- [@djdembeck](https://github.com/djdembeck) - Idea & Initial work

## 🎉 Acknowledgements <a name = "acknowledgement"></a>

- [sandreas](https://github.com/sandreas) for creating and maintaining [m4b-tool](https://github.com/sandreas/m4b-tool)