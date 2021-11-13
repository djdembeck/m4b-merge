from pathlib import Path
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

# Test existence of mp4chaps
mp4chaps_bin = shutil.which('mp4chaps')
if not mp4chaps_bin:
    raise SystemExit(
        'Error: Cannot find mp4chaps binary.'
        )
