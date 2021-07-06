from pathlib import Path
import audible, getpass, html2text, logging
from datetime import datetime
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

    def parser(self):
        # Login or register as needed
        self.auth.handle_auth()
        aud_json = self.auth.client.get(
            f"catalog/products/{self.asin}",
            params={
                "response_groups": f'''
                contributors,
                product_desc,
                product_extended_attrs,
                product_attrs''',
                "asins": self.asin
            }
        )

        ### JSON RESPONSE
        # We have:
        # Summary, Title, Author, Narrator, Series
        # Want: series number

        # metadata dictionary
        metadata_dict = {}

        ## Title
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

        ## Short summary
        aud_short_summary_json = (
            aud_json['product']['merchandising_summary']
            )
        metadata_dict['short_summary'] = (
            html2text.html2text(aud_short_summary_json).replace("\n", " ").replace("\"", "'")
            )

        ## Long summary
        aud_long_summary_json = (
            aud_json['product']['publisher_summary']
            )
        metadata_dict['long_summary'] = aud_long_summary_json

        ## Authors
        aud_authors_json = (
            aud_json['product']['authors']
            )
        # check if list contains more than 1 author
        if len(aud_authors_json) > 1:
            aud_authors_arr = []
            for author in range(len(aud_authors_json)):
                # Use ASIN for author only if available
                if aud_authors_json[author].get('asin'):
                    # from array of dicts, get author name
                    aud_authors_arr.append(
                    {
                    'asin': aud_authors_json[author]['asin'],
                    'name': aud_authors_json[author]['name']
                    }
                        )
                else:
                    aud_authors_arr.append(
                    {
                    'name': aud_authors_json[author]['name']
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
        
        ## Narrators
        aud_narrators_json = (
            aud_json['product']['narrators']
            )
        # check if list contains more than 1 narrator
        if len(aud_narrators_json) > 1:
            aud_narrators_arr = []
            for narrator in range(len(aud_narrators_json)):
                # from array of dicts, get narrator name
                aud_narrators_arr.append(
                    aud_narrators_json[narrator]['name']
                    )
            metadata_dict['narrators'] = aud_narrators_arr
        else:
            # else narrator name will be in first element dict
            metadata_dict['narrators'] = (
                [aud_narrators_json[0]['name']]
                )

        ## Series
        # Check if book has publication name (series)
        if 'publication_name' in aud_json['product']:
            metadata_dict['series'] = (
                aud_json['product']['publication_name']
                )

        ## Release date
        if 'release_date' in aud_json['product']:
            # Convert date string into datetime object
            metadata_dict['release_date'] = (
                datetime.strptime(
                    aud_json['product']['release_date'], '%Y-%m-%d'
                    ).date()
                )

        ## Publisher
        if 'publisher_name' in aud_json['product']:
            metadata_dict['publisher_name'] = aud_json['product']['publisher_name']

        ## Language
        if 'language' in aud_json['product']:
            metadata_dict['language'] = aud_json['product']['language']

        ## Runtime in minutes
        if 'runtime_length_min' in aud_json['product']:
            metadata_dict['runtime_length_min'] = aud_json['product']['runtime_length_min']

        ## Format type (abridged or unabridged)
        if 'format_type' in aud_json['product']:
            metadata_dict['format_type'] = aud_json['product']['format_type']

        # return all data
        return metadata_dict