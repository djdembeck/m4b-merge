#!/bin/bash
# Script to use m4b-tool to merge audiobooks, easily.
## REQUIRES: bash, curl, GNU grep, GNU iconv, mediainfo, pv, https://github.com/sandreas/m4b-tool
VER=1.5.8

### USER EDITABLE VARIABLES ###

#LOCAL FOLDERS
OUTPUT=""

# Desired bitrate, e.g. 64k, 128k, ... [default: ""]
# Default will scan and keep source bitrate
# Script will use this value if no environment variable is detected.
if [[ -z $GLOBALBITRATE ]]; then
	# EDIT THIS
	GLOBALBITRATE=""
fi

# Desired samplerate, e.g. 22.05 or 44.1
# Default will scan and keep source samplerate
# Script will use this value if no environment variable is detected.
if [[ -z $GLOBALSAMPLERATE ]]; then
	# EDIT THIS
	GLOBALSAMPLERATE=""
fi

# Command for m4b-tool, can be full path or just alias/command.
M4BPATH=""

# Path to cookies file for audible
# Used for Audible (curl) lookup requests
# Uses Netscape cookie file format
AUDCOOKIES=""

# Override job count. Default uses number of available CPU threads
JOBCOUNT=""

### END USER EDITABLE VARIABLES ###

# Check if there's no /output folder from docker
if [[ ! -d /output ]]; then
	# Check if output env var is empty
	if [[ -z $OUTPUT ]]; then
		echo "Output is not set. Exiting."
		exit 1
	fi
else
	OUTPUT="/output"
fi

# If nothing is set, assume default m4b-tool location
if [[ -z $M4BPATH ]]; then
	M4BPATH="$(which m4b-tool)"
fi

# If no path is set for cookies, set default
if [[ -z $AUDCOOKIES ]]; then
	AUDCOOKIES="/tmp/aud-cookies.txt"
fi

# If no manual override for jobs, use number of available CPU threads
if [[ -z $JOBCOUNT ]]; then
	JOBCOUNT="$(grep -c ^processor /proc/cpuinfo)"
fi

# -h help text to print
usage="	$(basename "$0") $VER [-b] [-f] [-h] [-m] [-r] [-s] [-v] [-y]

	'-b' Batch mode. File input is used for 1 folder only.
	'-f' File or folder to run from. Enter multiple files if you need, as: -f file1 -f file2 -f file3
	'-h' This help text.
	'-m' Use manual metadata mode instead of Audible metadata fetching.
	'-r' Bitrate (64k, 128k, etc).
	'-s' Samplerate (22.05 or 44.1).
	'-v' Verbose mode.
	'-y' Answer 'yes' to all prompts.
	"

# Flags for this script
	while getopts ":bf:hmr:s:vy" option; do
 case "${option}" in
	b) BATCHMODE=true
		;;
	f) FILEIN+=("$(realpath "$OPTARG")")
		;;
	h) echo "$usage"
 		exit
		;;
	m) AUDIBLEMETA=false
		;;
	r) LOCALBITRATE="$OPTARG"
		;;
	s) LOCALSAMPLERATE="$OPTARG"
		;;
	v) VERBOSE=true
		;;
	y) YPROMPT=true
		;;
 \?) echo -e "\e[91mInvalid flag: -"$OPTARG". Use '-h' for help.\e[0m" >&2
 	;;
 :) echo -e "\e[91mOption -$OPTARG requires a value.\e[0m" >&2
      exit 1
	;;

 esac
done

### Functions ###

function preprocess() {
	# Let's first check that the input folder, actually should be merged.
	# Import metadata into an array, so we can use it.
	importmetadata

	#Final chapter path to use
	ENDCHPTFILE="${OUTPUT}/${albumartistvar}/${albumvar}/${namevar}.chapters.txt"

	# Common extensions for audiobooks.
	EXTENSION="$(find "$SELDIR" -maxdepth 2 -type f -regex ".*\.\(m4a\|mp3\|m4b\)" | sort | head -n1)"
	EXT="${EXTENSION##*.}"

	# Check whether directory has multiple audio files or not
	if [[ -d $SELDIR && $(find "$SELDIR" -name "*.$EXT" | wc -l) -gt 1 ]] || [[ -f $SELDIR && $EXT == "mp3" ]]; then
		notice "Directory with multiple files"

		readarray M4BSEL <<<"$(cat "${M4BSELFILE::-4}".bit.txt | tr ' ' '\n' | tr '_' ' ')"
        # Add bitrate/samplerate commands to command pool, since we are merging
        # After we verify the input needs to be merged, lets run the merge command.
		pipe "$M4BPATH" merge \
		--output-file "$OUTPUT"/"$albumartistvar"/"$albumvar"/"$namevar".m4b \
		"${M4BSEL[@]//$'\n'/}" \
		--force \
		--no-chapter-reindexing \
		--no-cleanup \
		--jobs="$JOBCOUNT" \
		"$SELDIR"

		# Standardize chapters
		processchapters

		color_highlight "Merge completed for $namevar."

	# Folders with single m4b files
	elif [[ -d $SELDIR && $(find "$SELDIR" -name "*.$EXT" | wc -l) -eq 1 && $EXT == "m4b" ]]; then
		notice "Single m4b in a folder"
		SELDIR="$(find "$SELDIR" -name "*.$EXT")"
		# After we verify the type of input is a single m4b in a folder
		notice "Exporting chapterfile"
		# Get chapters from existing m4b file
		"$M4BPATH" meta \
		--export-chapters "" \
		"$SELDIR"
		mv "${SELDIR::-4}".chapters.txt "$ENDCHPTFILE"

		# run meta change commands only, then copy
		notice "Making backup file"
        cp "$SELDIR" "${SELDIR::-4}.new.m4b"
		"$M4BPATH" meta \
		"${M4BSEL[@]//$'\n'/}" \
		"${SELDIR::-4}.new.m4b"

		notice "Moving modified file to final output"
		mv "${SELDIR::-4}.new.m4b" "$OUTPUT"/"$albumartistvar"/"$albumvar"/"$namevar".m4b

		# Fix chapters and re-import them
		processchapters

	# Single m4b files
	elif [[ -f $SELDIR && $EXT == "m4b" ]]; then
		notice "Single m4b file"
		# After we verify the type of input is a single m4b
		notice "Exporting chapterfile"
		# Get chapters from existing m4b file
		"$M4BPATH" meta \
		--export-chapters "" \
		"$SELDIR"
		mv "${SELDIR::-4}".chapters.txt "$ENDCHPTFILE"

		# run meta change commands only, then copy
		notice "Making backup file"
        cp "$SELDIR" "${SELDIR::-4}.new.m4b"
		"$M4BPATH" meta \
		"${M4BSEL[@]//$'\n'/}" \
		"${SELDIR::-4}.new.m4b"
		notice "Moving modified file to final output"
		mv "${SELDIR::-4}.new.m4b" "$OUTPUT"/"$albumartistvar"/"$albumvar"/"$namevar".m4b

		# Fix chapters and re-import them
		processchapters
	elif [[ -z $EXT ]]; then
		error "No recognized filetypes found for $namevar."
		warn "Skipping..."
	fi
}

function processchapters() {
	# Working around chapter re-indexing being broken:
	# https://github.com/sandreas/m4b-tool/issues/105#issuecomment-730825979

	# Verify chapter file exists and has data
	if [[ -s $ENDCHPTFILE ]]; then
		# Run after merge, then re-import new chapters
		notice "Standardizing chapter file"

		# Don't modify first line of chapter file
		# Replace everything after spaces->endline with 'Chapter 01-9999'
		awk 'NR>1 { gsub(/ .*/, " Chapter " sprintf("%02d",++i)) } 1' "$ENDCHPTFILE" > "${ENDCHPTFILE::-3}1.txt" || error "Failed to standardize chapters"
		mv "${ENDCHPTFILE::-3}1.txt" "$ENDCHPTFILE"

		# Edit meta of newly merged m4b in-place
		mp4chaps --quiet -i \
		"$OUTPUT"/"$albumartistvar"/"$albumvar"/"$namevar".m4b
	fi
}

function audibleparser() {
	AUDMETAFILE="/tmp/.audmeta.$BASESELDIR.txt"

	if [[ -s $AUDMETAFILE ]]; then # Check if we can use existing audible data
		color_highlight "Using cached Audible metadata for $BASESELDIR"
	elif [[ ! -s $AUDMETAFILE ]]; then # Check if we can use an existing metadata entry
		RET=1
		until [[ $RET -eq 0 ]]; do
			echo ""
			color_action "Enter Audible ASIN for $BASESELDIR"
			read -e -p 'ASIN: ' ASIN

			CHECKASIN="$(curl -o /dev/null -L --silent --head --write-out '%{http_code}\n' https://www.audible.com/pd/$ASIN)"
			RET=$?

			if [[ -z $ASIN ]]; then
				error "No ASIN was entered. Try again."
				RET=1
			elif [[ $CHECKASIN != "200" ]]; then
				error "Could not access ASIN for $BASESELDIR (Was it entered correctly?)"
				RET=1
			elif [[ $CHECKASIN == "200" ]]; then
				RET=0
			fi
		done
		if [[ ! -s $AUDCOOKIES ]]; then
			error "Cookie file missing. This may lead to certain elements not working (like series and book numbering)"
		fi
		color_action "Fetching metadata from Audible..."
		curl -L -H "User-Agent: Mozilla/4.0 (compatible; MSIE 8.0; Windows NT 6.2; Trident/4.0; SLCC2; .NET CLR 2.0.50727; .NET CLR 3.5.30729; .NET CLR 3.0.30729; Media Center PC 6.0)" --cookie $AUDCOOKIES https://www.audible.com/pd/$ASIN -s -o "$AUDMETAFILE"
	fi

	unset useoldmeta

	# Check for multiple narrators
	NARRCMD="$(grep "searchNarrator=" "$AUDMETAFILE" | grep c1_narrator | grep -o -P '(?<=>).*(?=<)' | sort -u | iconv -f UTF-8 -t ascii//TRANSLIT)"
	if [[ $(echo "$NARRCMD" | wc -l) -gt 1 ]]; then
		notice "Correcting formatting for multiple narrators..."
		NUM="$(echo "$NARRCMD" | wc -l)"
		NARRCMD="$(cat "$AUDMETAFILE" | grep "searchNarrator=" | grep c1_narrator | grep -o -P '(?<=>).*(?=<)' | sort -u | sed -e "2,${NUM}{s#^#, #}" | tr -d '\n' | iconv -f UTF-8 -t ascii//TRANSLIT)"
	fi
	AUTHORCMD="$(grep "/author/" "$AUDMETAFILE" | grep -o -P '(?<=>).*(?=<)' | head -n1 | iconv -f UTF-8 -t ascii//TRANSLIT)"
	# Prefer being strict about authors, unless we can't find them.
	if [[ -z $AUTHORCMD ]]; then
		notice "Could not find author using default method. Trying backup method..."
		AUTHORCMD="$(cat "$AUDMETAFILE" | grep "author" | grep -o -P '(?<=>).*(?=<)' | head -n1 | iconv -f UTF-8 -t ascii//TRANSLIT)"
	fi
	TICTLECMD="$(grep "title"  "$AUDMETAFILE" | grep "content=" -m 1 | head -n1 | grep -o -P '(?<=content=").*(?=")' | sed -e 's/[[:space:]]*$//' | iconv -f UTF-8 -t ascii//TRANSLIT)"
	SERIESCMD="$(grep "/series" "$AUDMETAFILE" -m 1 | grep -o -P '(?<=>).*(?=<)' | iconv -f UTF-8 -t ascii//TRANSLIT)"
	if [[ $(echo "$SERIESCMD" | grep "chronological" | wc -l) -ge 1 ]]; then
		notice "Detected 2 book orders. Using Chronological order."
		SERIESCMD="$(grep "chronological" -m 1 "$AUDMETAFILE" | grep -o -P '(?<=>).*(?=,)' | sed -e 's#</a>##' | iconv -f UTF-8 -t ascii//TRANSLIT)"
		if [[ $(echo "$SERIESCMD" | grep "Book" | wc -l) -lt 1 ]]; then
			notice "Detected possible issue with Book number missing. Being less strict to retrieve it."
			SERIESCMD="$(grep "chronological" -m 1 "$AUDMETAFILE" | grep -o -P '(?<=>).*(?=)' | sed -e 's#</a>##' | iconv -f UTF-8 -t ascii//TRANSLIT)"
		fi
	fi
	BOOKNUM="$(grep "/series" -A 1 -m 1 "$AUDMETAFILE" | grep -o -P '(?<=>).*(?=)' | cut -d ',' -f 2 | sed -e 's/^[[:space:]]*//' | iconv -f UTF-8 -t ascii//TRANSLIT)"
	# Don't include book number, if it doesn't actually say which book it is
	if [[ $(echo "$BOOKNUM" | grep "Book" | wc -l ) -lt 1 ]] || [[ $(echo "$BOOKNUM" | grep "Book" | wc -l ) -gt 1 ]]; then
		notice "Detected either no book number, or more than 1 book number."
		BOOKNUM=""
	fi
	SUBTITLE="$(grep "subtitle" -m 1 -A 5 "$AUDMETAFILE" | tail -n1 | sed -e 's/^[[:space:]]*//' | iconv -f UTF-8 -t ascii//TRANSLIT | tr -dc '[:print:]')"
	if [[ -n "$SERIESCMD" && $(echo "$SUBTITLE" | grep "$(echo "$SERIESCMD" | cut -d ' ' -f 1-2)" | wc -l) -ge 1 ]]; then
		notice "Subtitle appears to be the same or similar to series name. Excluding the subtitle."
		SUBTITLE=""
	fi

	# Extract plain number from Book number
	SERIESNUMBER="$(echo "$BOOKNUM" | sed 's|[^0-9]||g')"

	# Check what metadata we can actually use for the title/name
	m4bvar1="$TICTLECMD" # Default
	if [[ -n $SERIESCMD && -n $BOOKNUM && -z "$SUBTITLE" ]]; then
		m4bvar1="$TICTLECMD ($SERIESCMD, $BOOKNUM)"
	elif [[ -z $SERIESCMD && -z $BOOKNUM && -n "$SUBTITLE" ]]; then
		m4bvar1="$TICTLECMD - $SUBTITLE"
	elif [[ -n $SERIESCMD && -z $BOOKNUM && -z "$SUBTITLE" ]]; then
		m4bvar1="$TICTLECMD ($SERIESCMD)"
	elif [[ -n $SERIESCMD && -z $BOOKNUM && -n "$SUBTITLE" ]]; then
		m4bvar1="$TICTLECMD - $SUBTITLE ($SERIESCMD)"
	elif [[ -n $SERIESCMD && -n $BOOKNUM && -n $SUBTITLE ]]; then
		# Don't include subtitle text if it is just saying what book in the series it is.
		if [[ "$(echo "$SUBTITLE" | grep "$BOOKNUM" | wc -l)" -eq 0 ]]; then
			m4bvar1="$TICTLECMD - $SUBTITLE ($SERIESCMD, $BOOKNUM)"
		else
			m4bvar1="$TICTLECMD ($SERIESCMD, $BOOKNUM)"
		fi
	fi

	m4bvar2="$TICTLECMD"
	m4bvar3="$NARRCMD"
	m4bvar4="$AUTHORCMD"

	makearray
	makearray2

	color_highlight "Metadata parsed as ( Title | Album | Narrator | Author ):"
	color_highlight "$m4bvar1 | $m4bvar2 | $m4bvar3 | $m4bvar4"
	echo ""
}

function makearray() {
	# Put all values into an array
	M4BARR=(
	"--name"
	"${m4bvar1// /_}"
	"--album"
	"${m4bvar2// /_}"
	"--artist"
	"${m4bvar3// /_}"
	"--albumartist"
	"${m4bvar4// /_}"
	)

    # Check that series value exists and add to array
    if [[ -n $SERIESCMD ]]; then
		notice "Series being set"
        M4BARR+=(
        "--series"
        "${SERIESCMD// /_}"
        )
    fi

    if [[ -n $SERIESNUMBER ]]; then
		notice "Series part being set"
        M4BARR+=(
        "--series-part"
        "${SERIESNUMBER// /_}"
        )
    fi

	if [[ -n $mbid ]]; then
		notice "MBID being set"
		M4BARR+=(
		"--musicbrainz-id"
		"${mbid// /_}"
		)
	fi

	# Make array into file
	echo -n "${M4BARR[*]}" > "$M4BSELFILE"
}

function makearray2() {
    # Put all values into an array
		notice "Adding bitrate/samplerate commands"
		if [[ -n $bitrate ]]; then
		    M4BARR2=(
			"--audio-bitrate"
		    "${bitrate// /_}"
			)
		fi
		if [[ -n $samplerate ]]; then
			M4BARR2+=(
			"--audio-samplerate"
		    "${samplerate// /_}"
		    )
		fi

	    # Append array into file
	    echo "${M4BARR2[*]}" > "${M4BSELFILE::-4}".bit.txt
}

function collectmeta() {
	if [[ $BATCHMODE == "true" && $(echo "${FILEIN[@]}" | wc -l) -eq 1 ]]; then
		# This will recursively go through the input folder
		MULTIORNAH="/*"
	fi
	for SELDIR in "${FILEIN[@]}"$MULTIORNAH; do
		# Basename of array values
		BASESELDIR="$(basename "$SELDIR")"
		M4BSELFILE="/tmp/.m4bmerge.$BASESELDIR.txt"

		# Common extensions for audiobooks.
		EXTENSION="$(find "$SELDIR" -maxdepth 2 -type f -regex ".*\.\(m4a\|mp3\|m4b\)" | sort | head -n1)"
		EXT="${EXTENSION##*.}"

		if [[ -z $EXT ]]; then
			error "File extension unkown for $BASESELDIR
			"
		else
			notice "---- START RATE INFO ----"

			# Get bitrate data/command
			# Prefer order: Bitrate from flag -r->Global Bitrate-> None specified
			if [[ -n $LOCALBITRATE ]]; then
				bitrate="$LOCALBITRATE"
				notice "Using flag-defined bitrate of $LOCALBITRATE"
			elif [[ -n $GLOBALBITRATE ]]; then
				bitrate="$GLOBALBITRATE"
				notice "Using global bitrate of $GLOBALBITRATE"
			elif [[ -z $GLOBALBITRATE ]]; then
				FNDFIRST="$(find "$SELDIR" -name "*.$EXT" | sort | head -1)"
				FNDBITRATE="$(mediainfo "$FNDFIRST" | grep 'Overall bit rate                         : ' | cut -d ':' -f 2 | tr -d ' ' | cut -d 'k' -f 1 | cut -d '.' -f 1)"
				bitrate="${FNDBITRATE}k"
				notice "Audio bitrate set to ${FNDBITRATE}k"
			fi

			# Get origin samplerate to match
			# Prefer order: Bitrate from flag -r->Global Bitrate-> None specified
			if [[ -n $LOCALSAMPLERATE ]]; then
				FNDSAMPLERATE="$LOCALSAMPLERATE"
			elif [[ -n $GLOBALSAMPLERATE ]]; then
				FNDSAMPLERATE="$GLOBALSAMPLERATE"
			elif [[ -z $GLOBALSAMPLERATE ]]; then
				FNDSAMPLERATE="$(mediainfo "$FNDFIRST" | grep 'Sampling rate                            : ' | cut -d ':' -f 2 | tr -d ' ' | cut -d 'k' -f 1)"
			fi

			# Get samplerate
			# If no samplerate, throw error
			if [[ -z $FNDSAMPLERATE ]]; then
				error "No Samplerate could be determined"
				NORATE=true
			else
				# Multiply by 1000 to get khz value
				FNLSAMPLERATE=$(echo "1000 * $FNDSAMPLERATE" | bc -l | cut -d '.' -f 1)
			fi

			if [[ $NORATE != "true" ]]; then
				notice "Audio samplerate set to ${FNDSAMPLERATE}khz"
				# Final variable for array
				samplerate="${FNLSAMPLERATE}"
			fi
			NORATE=false

			notice "---- END OF RATE INFO----
			"

			if [[ $AUDIBLEMETA != "false" ]]; then
				audibleparser
			else
				if [[ $YPROMPT == "true" ]]; then
					useoldmeta="y"
				elif [[ -s $M4BSELFILE ]]; then # Check if we can use an existing metadata entry
					color_highlight "Metadata for $BASESELDIR exists"
					read -e -p 'Use existing metadata? y/n: ' useoldmeta
				elif [[ ! -f $M4BSELFILE ]]; then # Check if we can use an existing metadata entry
					useoldmeta="n"
				fi

				if [[ $useoldmeta == "n" ]]; then
					color_highlight "Enter metadata for $BASESELDIR"
					# Each line has a line after input, adding that value to an array.
					read -e -p 'Enter name: ' m4bvar1
					read -e -p 'Enter Albumname: ' m4bvar2
					read -e -p 'Enter artist (Narrator): ' m4bvar3
					read -e -p 'Enter albumartist (Author): ' m4bvar4
					read -e -p 'Enter Musicbrainz ID, if any (leave blank for none): ' m4bvar6

					# Check if we need to include optional arguments in the array
					if [[ -z $m4bvar6 ]]; then
						mbid=""
					else
						mbid="$m4bvar6"
					fi

					# Call array function
					makearray
				elif [[ -s $M4BSELFILE && $useoldmeta == "y" ]]; then
					color_highlight "Using this metadata then:"
					color_highlight "$(cat "$M4BSELFILE" | tr '_' ' ')"
					echo ""
				fi
			fi
			# Warn user (without input to stop) if destination exists, and give some stats
			if [[ -d "$OUTPUT"/"$m4bvar4"/"$m4bvar2" ]]; then
				warn "^^^^ START DUPLICATE NOTICE ^^^^"
				FNDDUPES="$(find "$OUTPUT"/"$m4bvar4"/"$m4bvar2" -maxdepth 1 -not -name '*.txt' -type f)"
				if [[ "$(echo "$FNDDUPES" | wc -l)" -eq 1 ]]; then
					warn "Destination file exists (and will be overwritten) for $m4bvar2"
					OLDSAMPLERATE="$(mediainfo "$OUTPUT"/"$m4bvar4"/"$m4bvar2"/* | grep 'Sampling rate                            : ' | cut -d ':' -f 2 | tr -d ' ' | cut -d 'k' -f 1)"
					OLDBITRATE="$(mediainfo "$OUTPUT"/"$m4bvar4"/"$m4bvar2"/* | grep 'Overall bit rate                         : ' | cut -d ':' -f 2 | tr -d ' ' | cut -d 'k' -f 1 | cut -d '.' -f 1)"

					warn "Existing bitrate: ${OLDBITRATE}k"
					warn "Existing samplerate: ${OLDSAMPLERATE}khz"
				elif [[ "$(echo "$FNDDUPES" | wc -l)" -gt 1 ]]; then
					warn "Multiple destination files exist for $m4bvar2"
					warn "This can happen if metadata changes or multiple versions of a book exist."
					notice "Logging duplicate books to $OUTPUT/dupes.txt"
					echo "$FNDDUPES" >> "$OUTPUT"/dupes.txt
					sort -u "$OUTPUT"/dupes.txt > "$OUTPUT"/dupes.txt.new
					mv "$OUTPUT"/dupes.txt.new "$OUTPUT"/dupes.txt
				fi
				warn "---- END OF DUPLICATE NOTICE ----
				"
			fi
		fi
	done
}

function importmetadata() {
	# Basename of array values
	BASESELDIR="$(basename "$SELDIR")"
	M4BSELFILE="/tmp/.m4bmerge.$BASESELDIR.txt"

	# Import values from file into array.
	readarray M4BSEL <<<"$(cat "$M4BSELFILE" | tr ' ' '\n' | tr '_' ' ')"
	namevar="$(echo "${M4BSEL[1]}" | sed s/\'//g)"
	albumvar="$(echo "${M4BSEL[3]}" | sed s/\'//g)"
	artistvar="$(echo "${M4BSEL[5]}" | sed s/\'//g)"
	albumartistvar="$(echo "${M4BSEL[7]}" | sed s/\'//g)"
}

function batchprocess() {
	if [[ $BATCHMODE == "true" && $(echo "${FILEIN[@]}" | wc -l) -eq 1 ]]; then
		# This will recursively go through the input folder
		INPUTNUM="$(ls "${FILEIN[@]}" | wc -l)"
	else
		INPUTNUM="${#FILEIN[@]}"
	fi
	((COUNTER++))
	# Output number of folders to process
	color_action "Let's begin processing input folders"
	color_highlight "Number of folders to process: $INPUTNUM"

	for SELDIR in "${FILEIN[@]}"$MULTIORNAH; do
		# Basename of array values
		BASESELDIR="$(basename "$SELDIR")"
		M4BSELFILE="/tmp/.m4bmerge.$BASESELDIR.txt"

		# Common extensions for audiobooks.
		EXTENSION="$(find "$SELDIR" -maxdepth 2 -type f -regex ".*\.\(m4a\|mp3\|m4b\)" | sort | head -n1)"
		EXT="${EXTENSION##*.}"

		if [[ -z $EXT ]]; then
			error "File extension unkown for $BASESELDIR
			"
		else
			# Import metadata into an array, so we can use it.
			importmetadata

			# Make sure output file exists as expected
			if [[ -s $M4BSELFILE ]]; then
				mkdir -p "$OUTPUT"/"$albumartistvar"/"$albumvar"
				color_action  "($COUNTER of $INPUTNUM): Processing $albumvar..."

				# Process input, and determine if we need to run merge, or just cleanup the metadata a bit.
				preprocess
				((COUNTER++))

				# Check if m4b-tool made leftover files
				if [[ "$(find "$OUTPUT"/"$albumartistvar"/"$albumvar" -maxdepth 1 -name '*-tmpfiles' -type d | wc -l)" -eq 1 ]]; then
					rm -rf "$OUTPUT"/"$albumartistvar"/"$albumvar"/*-tmpfiles
				fi
			else
				error "metadata file for $BASESELDIR does not exist"
				exit 1
			fi
		fi
	done
}

### Style functions ###
function notice () {
    if [[ $VERBOSE == "true" ]]; then
        echo -e "\e[34mNOTICE: $@\e[0m"
    fi
}

function warn () {
    if [[ $VERBOSE == "true" ]]; then
        echo -e "\e[33mWARN: $@\e[0m"
    fi
}

function error () {
	# Color and text for error echoes
    echo -e "\e[91mERROR: $@\e[0m"
}

function color_highlight () {
	# Color and text for error echoes
    echo -e "\e[96m$@\e[0m"
}

function color_action () {
	# Color and text for error echoes
    echo -e "\e[95m$@\e[0m"
}

function pipe() {
	# Function to replace output text with pv.
	if [[ $VERBOSE == "true" ]]; then
		"$@"
	else
		"$@" 2> /dev/null | pv -l -p -t -N "Processing" > /dev/null
	fi
}

function silenterror() {
	if [[ $VERBOSE == "true" ]]; then
		"$@"
	else
		"$@" 2> /dev/null
	fi
}
#### End functions ####

notice "Verbose mode is ON"

#### Checks ####
# Make sure user gave usable INPUT
if [[ -z "${FILEIN[@]}" ]]; then
	error "No file inputs given."
	echo "$usage"
	exit 1
fi

## Check dependencies
notice "Begin dependencies check"

if [[ -z $(which bash) ]]; then
	error "Bash is not installed"
	exit 1
fi

if [[ -z $(which curl) ]]; then
	error "Curl is not installed"
	exit 1
fi

if [[ -z $(which mediainfo) ]]; then
	error "Mediainfo is not installed"
	exit 1
fi

if [[ -z $(which pv) ]]; then
	error "PV is not installed"
	exit 1
fi
notice "End dependencies check"
## End dependencies check

# verify m4b command works properly
if [[ -z $M4BPATH ]]; then
	error "No m4b-tool installation detected. Exiting"
	exit 1
elif [[ -n $($M4BPATH -h) ]]; then
	if [[ $? -ne 0 ]]; then
		error "Could not successfully run m4b-tool, exiting."
		exit 1
	fi
fi
notice "Got $JOBCOUNT processors to use"
#### End checks ####

# Gather metadata from user (Audible/manual input prompt)
collectmeta

# Process metadata batch (Main processing command)
batchprocess

color_highlight "Script complete."
