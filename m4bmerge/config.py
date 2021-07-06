from pathlib import Path

# config section for docker
if Path('/config').is_dir():
	dir_path = Path('/config')
else:
	dir_path = Path(f"{Path(__file__).resolve().parent.parent}/config")
	Path(dir_path).mkdir(
	parents=True,
	exist_ok=True
	)