from pathlib import Path
import logging
import shutil
from appdirs import user_config_dir

# config section for docker
if Path('/config').is_dir():
    config_path = Path('/config')
else:
    appname = "m4b-merge"
    config_path = Path(user_config_dir(appname))
    Path(config_path).mkdir(
        parents=True,
        exist_ok=True
    )

# Find path to m4b-tool binary
# Check that binary actually exists
m4b_tool_bin = shutil.which('m4b-tool')
if not m4b_tool_bin:
    raise SystemExit(
        'Error: Cannot find m4b-tool binary.'
        )

# Find tone binary (OPTIONAL during migration)
tone_bin = shutil.which('tone')
if not tone_bin:
    logging.warning('tone binary not found. Metadata operations will use m4b-tool.')

# Test existence of mp4chaps (OPTIONAL - for Ubuntu 22.04+ compatibility)
mp4chaps_bin = shutil.which('mp4chaps')
if not mp4chaps_bin:
    logging.warning('mp4chaps binary not found. Chapter operations will use alternative method.')
