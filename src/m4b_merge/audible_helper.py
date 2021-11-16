import logging
import requests
from datetime import timedelta


# Checks validity of asin, then gathers json response into a return object
class BookData:
    def __init__(self, asin):
        self.asin = asin

    # Convert MS to timestamp format hh:mm:ss.ms
    def ms_to_timestamp(self, input_duration):
        conversion = timedelta(milliseconds=input_duration)
        # Hacky fix for timedelta showing days past 24hr
        # Test if we've gone into days
        if "day" in str(conversion):
            # Find hour after days string
            hour = int(str(conversion).split(", ")[1].split(":")[0])
            # Find int number of days
            day = int(str(conversion).split(" ")[0])
            # Add 24 hours for each day
            real_hours = (hour+(day*24))
            # Get the rest of the time string based on colon split
            remainder = str(conversion).split(", ")[1].split(":", 1)[1]
            timestamp = f"{real_hours}:{remainder}"
        else:
            timestamp = str(conversion)

        # Remove trailing 000 if it makes ms 6 places long
        if "." in timestamp:
            split_timestamp = timestamp.split(".")
            prefix = split_timestamp[0]
            suffix = split_timestamp[1]
            if len(suffix) > 3:
                suffix = suffix.rstrip("000")
            return prefix + '.' + suffix

        return timestamp + '.' + '000'

    def get_chapters(self):
        # Select chapter data from json response
        chapter_info = self.metadata_dict['chapter_info']

        # Only use Audible chapters if tagged as accurate
        if 'isAccurate' in chapter_info and chapter_info['isAccurate'] is True:
            chapter_output = []
            # Append total runtime to the top of file
            total_len = self.ms_to_timestamp(chapter_info['runtimeLengthMs'])
            chapter_output.append(
                (
                    "# total-length"
                    f" {total_len}"
                )
            )

            # Append each chapter to array
            for chapter in chapter_info['chapters']:
                chap_start = self.ms_to_timestamp(chapter['startOffsetMs'])
                # Starting chapter title data
                chapter_title = chapter['title']
                chapter_output.append(
                    (
                        f"{chap_start}"
                        f" {chapter_title}"
                    )
                )
        else:
            logging.warning(
                "Not using Audible chapters as they aren't tagged as accurate"
            )
            chapter_output = None

        return chapter_output

    def fetch_api_data(self, api_url):
        # metadata dictionary
        book_api_call = requests.get(
            f"{api_url}/books/{self.asin}"
        )
        chapter_api_call = requests.get(
            f"{api_url}/books/{self.asin}/chapters"
        )
        self.metadata_dict = book_api_call.json()
        self.metadata_dict['chapter_info'] = chapter_api_call.json()
        return self.metadata_dict
