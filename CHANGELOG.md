# Changelog

All notable changes to this project will be documented in this file. See [standard-version](https://github.com/conventional-changelog/standard-version) for commit guidelines.

### [0.4.5](https://github.com/djdembeck/m4b-merge/compare/v0.4.4...v0.4.5) (2021-12-06)


### Bug Fixes

* **merge:** :bug: handle api having no author or narrators ([3adac9b](https://github.com/djdembeck/m4b-merge/commit/3adac9bd66480e1b373f9a17946dbd6c355f1e9e))

### [0.4.4](https://github.com/djdembeck/m4b-merge/compare/v0.4.3...v0.4.4) (2021-11-26)


### Features

* **merge:** :sparkles: Allow specifying output naming convention ([8980308](https://github.com/djdembeck/m4b-merge/commit/89803080db9816b8a71b8ff2d1f5135c2199c4dc))


### Bug Fixes

* **merge:** :bug: don't create empty directory of file name ([cbd2297](https://github.com/djdembeck/m4b-merge/commit/cbd22973d137875a317d68dd444897f44ecb0830))
* **merge:** :bug: fix replace_tag replacing partial terms instead of full term ([7abea6f](https://github.com/djdembeck/m4b-merge/commit/7abea6fd5c08252e4413f42b83ca1ecff5a28479))

### [0.4.3](https://github.com/djdembeck/m4b-merge/compare/v0.4.1...v0.4.3) (2021-11-18)


### Features

* **merge:** :construction: better config 1: move user configurable options to arguments ([c2cd229](https://github.com/djdembeck/m4b-merge/commit/c2cd2292fc8d3b3d50511deaf404e3df487cfb86))


### Bug Fixes

* **audible:** :bug: fix double import config issue with api_url ([0e657fb](https://github.com/djdembeck/m4b-merge/commit/0e657fb0ae2a0a7d58dd53d72110d66e75dfef3b))
* **audible:** :bug: fix validate url ([36a357b](https://github.com/djdembeck/m4b-merge/commit/36a357bbfd030165c09a45e33baae17ee8c20d94))
* **audible:** :bug: pass url directly instead of importing config ([27f796f](https://github.com/djdembeck/m4b-merge/commit/27f796fb01f4d20bf9a12eafe7eb7fc5ff8430d6))
* **merge:** :ambulance: fix  inconsistent variable name ([51b9b94](https://github.com/djdembeck/m4b-merge/commit/51b9b94d1b96d073587a2cf760565cff479ab049))
* **merge:** :bug: fix asin validation before merge ([0d00c09](https://github.com/djdembeck/m4b-merge/commit/0d00c09d07322a34bd18d560e15bac333090bc67))
* **merge:** :bug: fix error when no cover exists ([b42b081](https://github.com/djdembeck/m4b-merge/commit/b42b081bdf28f4c526fedd8bd71870d8252481ea))
* **merge:** :bug: fix path comparison for junk dir ([a98c828](https://github.com/djdembeck/m4b-merge/commit/a98c8287069fbf90a075826848e2433225046992))

### [0.4.2](https://github.com/djdembeck/m4b-merge/compare/v0.4.1...v0.4.2) (2021-11-03)


### Bug Fixes

* **merge:** :bug: fix error when no cover exists ([b42b081](https://github.com/djdembeck/m4b-merge/commit/b42b081bdf28f4c526fedd8bd71870d8252481ea))

### [0.4.1](https://github.com/djdembeck/m4b-merge/compare/v0.3.5...v0.4.1) (2021-10-06)


### Bug Fixes

* **audible:** :bug: verify isAccurate exists before using it ([6f21eae](https://github.com/djdembeck/m4b-merge/commit/6f21eae6c343e14aafb1a4521444b1ad687c8184))
* **merge:** :bug: don't expect series position to exist ([cf41203](https://github.com/djdembeck/m4b-merge/commit/cf412030db3b9d2c67632f6ea1737c478bb3ad20))
* **merge:** :bug: set series_position to none if it doesn't exist ([3aaed08](https://github.com/djdembeck/m4b-merge/commit/3aaed08889f9585ad6b96a4a2f3434f7f0144f00))
