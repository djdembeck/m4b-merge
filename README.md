
## Usage
`m4b-merge.sh [-b] [-f] [-h] [-m] [-v] [-y]`

- `-b` Batch mode. File input is used for 1 folder only.
- `-f` File or folder to run from. Enter multiple files if you need, as: `-f file1 -f file2 -f file3`
- `-h` This help text.
- `-m` Use manual metadata mode instead of Audible metadata fetching.
- `-v` Verbose mode.
- `-y` Answer 'yes' to all prompts.

## Requirements
- [m4b-tool](https://github.com/sandreas/m4b-tool) by sandreas
    - [m4b-tool's list of dependencies](https://github.com/sandreas/m4b-tool#ubuntu)
- `bash`
- `curl`
- `mutagen`
- `pv`

## Configuration
This script requires little pre-configuration. Here's what you need to know/can change from the top of the file:

- `OUTPUT="/path/to/output"`
  
  By default this is empty. Personally I assign `OUTPUT` folder from my Docker image. If this is left blank, the script will assume `/output`

- `GLOBALBITRATE=""`

    Desired bitrate, e.g. `64k`, `128k`, ...
    
    Leaving this blank will default to `--no-conversion`, and will keep the source files' bitrate (lossless and thus recommended for higher quality, larger files).

    Read the m4b-tool [reference](https://github.com/sandreas/m4b-tool#reference) for more information.

- `M4BPATH="/path/to/m4b-tool"`

    For non-default executable locations.

- `AUDCOOKIES="/tmp/aud-cookies.txt"`

    Path to file containing Netscape cookie file for curl to use. This is used for special, member Audible pages.

- `JOBCOUNT="8"`

    By default, the script will determine available number of threads to use. This shouldn't be changed unless you want the script to use less than maximum available threads.

## Examples
I personally recommend you leverage the Audible data mode, as it is much less tenous than manually entering data. However, you may want data structured a certain way, Audible has it listed wrong, or there is a bug in this script. In that event, add the `-m` flag from the below examples for manual import mode.

### Batch importing
```
m4b-merge.sh -b -f /input
```
This will import everything under the folder `/input` in batch mode, using Audible metadata.

### Single importing
```
m4b-merge.sh -f /input/An\ Interesting\ Book
```
This will import only a single input (folder or single file, auto detected), using Audible metadata.

### Re-running batch import
```
m4b-merge.sh -b -y -f /input
```
This will import everything under the folder `/input` in batch mode, using already cached Audible metadata from previous imports.