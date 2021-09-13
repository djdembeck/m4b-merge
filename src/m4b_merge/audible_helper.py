from pathlib import Path
import audible
import getpass
import logging
import requests
from datetime import datetime, timedelta
# Local imports
from . import config


# Authenticates user if already setup or registers the user if not
class AudibleAuth:
    auth_file = Path(config.config_path, ".aud_auth.txt")

    def handle_auth(self):
        # If auth file doesn't exist, call register
        if not self.auth_file.exists():
            logging.error("Not logged in to Audible")
            self.register()
        else:
            self.authenticate()

    def authenticate(self):
        self.auth = audible.Authenticator.from_file(self.auth_file)
        self.client = audible.Client(self.auth)

    def custom_captcha_callback(self, captcha_url):
        logging.warning(
            "Open this URL in browser and then type your answer:"
        )
        print(captcha_url)

        self.CAPTCHA_GUESS = input("Captcha answer: ")
        return str(self.CAPTCHA_GUESS).strip().lower()

    def register(self):
        print("You need to login")
        self.USERNAME = input("Email: ")
        self.PASSWORD = getpass.getpass()
        auth = audible.Authenticator.from_login(
            self.USERNAME,
            self.PASSWORD,
            captcha_callback=self.custom_captcha_callback,
            locale="us",
            with_username=False,
            register=True
        )
        auth.to_file(self.auth_file)
        # Authenticate now that we have generated auth file
        self.authenticate()


# Checks validity of asin, then gathers json response into a return object
class BookData:
    audnexus_url = "https://api.audnex.us/books/"

    def __init__(self, asin):
        self.auth = AudibleAuth()
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
        self.auth.handle_auth()
        aud_chapter_json = self.auth.client.get(
            f"content/{self.asin}/metadata",
            params={
                "response_groups": "chapter_info"
            }
        )
        # Select chapter data from json response
        chapter_info = aud_chapter_json['content_metadata']['chapter_info']

        # Only use Audible chapters if tagged as accurate
        if chapter_info['is_accurate'] is True:
            chapter_output = []
            # Append total runtime to the top of file
            total_len = self.ms_to_timestamp(chapter_info['runtime_length_ms'])
            chapter_output.append(
                (
                    "# total-length"
                    f" {total_len}"
                )
            )

            # Append each chapter to array
            for chapter in chapter_info['chapters']:
                chap_start = self.ms_to_timestamp(chapter['start_offset_ms'])
                # Starting chapter title data
                original_title = chapter['title']
                stripped_title = original_title.rstrip('.')
                # Check if chapter title is purely numbers
                if stripped_title.isnumeric() and len(stripped_title) < 3:
                    # Remove trailing period in some cases
                    strip_period = stripped_title
                    # Convert to int to normalize numbers
                    int_title = int(strip_period)
                    # Convert back to string for file use
                    str_title = str(int_title)
                    logging.info(
                        f"Changing chapter: {original_title}"
                        f" -> Chapter {str_title}"
                    )
                    chapter_title = f"Chapter {str_title}"
                else:
                    chapter_title = original_title
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

    def parser(self):
        # metadata dictionary
        api_call = requests.get(f"{self.audnexus_url}{self.asin}")
        metadata_dict = api_call.json()
        print(metadata_dict)
        return metadata_dict

    def check_asin_sku(self):
        # Login or register as needed
        self.auth.handle_auth()
        aud_json = self.auth.client.get(
            f"catalog/products/{self.asin}",
            params={
                "response_groups": "sku",
                "asins": self.asin
            }
        )
        if 'sku' in aud_json['product']:
            return True
        return None
