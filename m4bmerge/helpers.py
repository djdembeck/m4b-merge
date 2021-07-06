import logging, requests

def validate_asin(asin):
	if len(asin) == 10:
		# Check that asin actually returns data from audible
		check = requests.get(f"https://www.audible.com/pd/{asin}")
		if check.status_code == 200:
			logging.info(f"Validated ASIN: {asin}")
		else:
			raise ValueError(f"HTTP error {check.status_code}")
	else:
		raise ValueError("Invalid ASIN length")