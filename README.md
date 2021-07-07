```
usage: m4b-merge [-h] -i INPUTS [INPUTS ...] [--log_level LOG_LEVEL]

m4b-merge cli

optional arguments:
  -h, --help            show this help message and exit
  -i INPUTS [INPUTS ...], --inputs INPUTS [INPUTS ...]
                        Input paths to process
  --log_level LOG_LEVEL
                        Set logging level
```
  - Check the user editable variables in [config.py](src/m4b_merge/config.py), and see if there's anything you need to change.

  - On first run, you will be prompted to signin to Audible. This is a one-time process that will be saved to the `config` folder.