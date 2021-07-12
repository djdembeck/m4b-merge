from pathlib import Path
import audible
import getpass
import html2text
import logging
from datetime import datetime, timedelta
# Local imports
from . import config


# Authenticates user if already setup or registers the user if not
class AudibleAuth:
    auth_file = Path(config.config_path, ".aud_auth.txt")

    def __init__(self, USERNAME="", PASSWORD=""):
        self.USERNAME = USERNAME
        self.PASSWORD = PASSWORD

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

    def register(self):
        print("You need to login")
        # Check if we're coming from web or not
        if not self.USERNAME:
            self.USERNAME = input("Email: ")
            self.PASSWORD = getpass.getpass()
        auth = audible.Authenticator.from_login(
            self.USERNAME,
            self.PASSWORD,
            locale="us",
            with_username=False,
            register=True
        )
        auth.to_file(self.auth_file)
        # Authenticate now that we have generated auth file
        self.authenticate()


# Checks validity of asin, then gathers json response into a return object
class BookData:
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

        # Remove trailing 000 except on opening credits
        if "." in timestamp:
            split_timestamp = timestamp.split(".")
            prefix = split_timestamp[0]
            suffix = split_timestamp[1].rstrip("000")
            return prefix + '.' + suffix
        return timestamp

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
        # Login or register as needed
        self.auth.handle_auth()
        aud_json = self.auth.client.get(
            f"catalog/products/{self.asin}",
            params={
                "response_groups": (
                    "contributors,"
                    "product_desc,"
                    "product_extended_attrs,"
                    "product_attrs,"
                    "media"),
                "asins": self.asin
            }
        )

        # JSON RESPONSE
        # We have:
        # Summary, Title, Author, Narrator, Series
        # Want: series number

        # metadata dictionary
        metadata_dict = {}

        # Title
        # Use subtitle if it exists
        if 'subtitle' in aud_json['product']:
            aud_title_start = aud_json['product']['title']
            aud_title_end = aud_json['product']['subtitle']
            metadata_dict['title'] = aud_title_start
            metadata_dict['subtitle'] = aud_title_end
        else:
            metadata_dict['title'] = (
                aud_json['product']['title']
                )

        # Short summary
        aud_short_summary_json = (
            aud_json['product']['merchandising_summary']
            )
        metadata_dict['short_summary'] = (
            html2text.html2text(aud_short_summary_json)
            .replace("\n", " ").replace("\"", "'")
            )

        # Long summary
        aud_long_summary_json = (
            aud_json['product']['publisher_summary']
            )
        metadata_dict['long_summary'] = aud_long_summary_json

        # Authors
        aud_authors_json = (
            aud_json['product']['authors']
            )
        # check if list contains more than 1 author
        if len(aud_authors_json) > 1:
            aud_authors_arr = []
            for author in aud_authors_json:
                # Use ASIN for author only if available
                if author['asin']:
                    # from array of dicts, get author name
                    aud_authors_arr.append(
                        {
                            'asin': author['asin'],
                            'name': author['name']
                        }
                    )
                else:
                    aud_authors_arr.append(
                        {
                            'name': author['name']
                        }
                    )
            metadata_dict['authors'] = aud_authors_arr
        else:
            # else author name will be in first element dict
            # Use ASIN for author only if available
            if aud_authors_json[0].get('asin'):
                metadata_dict['authors'] = [
                    {
                        'asin': aud_authors_json[0]['asin'],
                        'name': aud_authors_json[0]['name']
                    }
                ]
            else:
                metadata_dict['authors'] = [
                    {
                        'name': aud_authors_json[0]['name']
                    }
                ]

        # Narrators
        aud_narrators_json = (
            aud_json['product']['narrators']
            )
        # check if list contains more than 1 narrator
        if len(aud_narrators_json) > 1:
            aud_narrators_arr = []
            for narrator in aud_narrators_json:
                # from array of dicts, get narrator name
                aud_narrators_arr.append(
                    narrator['name']
                )
            metadata_dict['narrators'] = aud_narrators_arr
        else:
            # else narrator name will be in first element dict
            metadata_dict['narrators'] = (
                [aud_narrators_json[0]['name']]
            )

        # Series
        # Check if book has publication name (series)
        if 'publication_name' in aud_json['product']:
            metadata_dict['series'] = (
                aud_json['product']['publication_name'])

        # Release date
        if 'release_date' in aud_json['product']:
            # Convert date string into datetime object
            metadata_dict['release_date'] = (
                datetime.strptime(
                    aud_json['product']['release_date'], '%Y-%m-%d'
                    ).date()
                )

        # Publisher
        if 'publisher_name' in aud_json['product']:
            metadata_dict['publisher_name'] = (
                aud_json['product']['publisher_name'])

        # Language
        if 'language' in aud_json['product']:
            metadata_dict['language'] = (
                aud_json['product']['language'])

        # Runtime in minutes
        if 'runtime_length_min' in aud_json['product']:
            metadata_dict['runtime_length_min'] = (
                aud_json['product']['runtime_length_min'])

        # Format type (abridged or unabridged)
        if 'format_type' in aud_json['product']:
            metadata_dict['format_type'] = (
                aud_json['product']['format_type'])

        # Cover image
        if 'product_images' in aud_json['product']:
            metadata_dict['cover_image'] = (
                aud_json['product']['product_images']['500']
                .replace('_SL500_', '')
            )

        # return all data
        return metadata_dict
