# Changelog

All notable changes to this project will be documented in this file. See [standard-version](https://github.com/conventional-changelog/standard-version) for commit guidelines.

## 1.0.0 (2026-07-20)


### ⚠ BREAKING CHANGES

* correct chpl parsing, CI pipelines, license, and code quality
* CLI arguments converted from snake_case to kebab-case:
    - --api_url → --api-url
    - --completed_directory → --completed-directory
    - --num_cpus → --num-cpus
    - --log_level → --log-level
    - --path_format → --path-format

### Features

* complete Rust rewrite of m4b-merge ([#376](https://github.com/djdembeck/m4b-merge/issues/376)) ([6071f92](https://github.com/djdembeck/m4b-merge/commit/6071f92f36eab42a931e2e802c6d91cf73c7a224))
* improve batch processing resilience and add ffprobe_path to chapters ([01be42e](https://github.com/djdembeck/m4b-merge/commit/01be42ecbd72b7ba3b6dc38376b95c96d6e61178))
* **merge:** :construction: better config 1: move user configurable options to arguments ([ebe9349](https://github.com/djdembeck/m4b-merge/commit/ebe9349f074588b3d0681263360fe9ff8893df10))
* **merge:** :sparkles: add support for `asin` as output path term ([9d67a5a](https://github.com/djdembeck/m4b-merge/commit/9d67a5acdd8294380222a523cc25f51d792155fd))
* **merge:** :sparkles: Allow specifying output naming convention ([5837c8f](https://github.com/djdembeck/m4b-merge/commit/5837c8f844e38a5a68f8249014dc1a06a83fd46d))
* **merge:** :sparkles: use LOG_LEVEL from environment variable if available ([7df51c3](https://github.com/djdembeck/m4b-merge/commit/7df51c3ed994929ed7025861efd6194d4b8bdf0c))
* migrate Python m4b-merge to Rust ([1f94d30](https://github.com/djdembeck/m4b-merge/commit/1f94d3052b8b5acbe4b8346b13cdcfae4506f2ab))


### Bug Fixes

* **audible:** :bug: fix double import config issue with api_url ([1b1a6ae](https://github.com/djdembeck/m4b-merge/commit/1b1a6ae4405726bea8736099e44689367a395a6a))
* **audible:** :bug: fix validate url ([9a55bc4](https://github.com/djdembeck/m4b-merge/commit/9a55bc4d1ae497388d82d02f96f161904603918f))
* **audible:** :bug: pass url directly instead of importing config ([648c6e2](https://github.com/djdembeck/m4b-merge/commit/648c6e2f5c22fa50956e004650e08ff6400a2ef6))
* CI pipeline, Dockerfile, retry logic, chapter parsing, and dry-run mode ([513fba2](https://github.com/djdembeck/m4b-merge/commit/513fba208cffc1dd029be215eab8acd07ac1ca62))
* **ci:** correct setup-buildx-action commit SHA ([7e6d427](https://github.com/djdembeck/m4b-merge/commit/7e6d4270a516898bbecbff63fc6fbbae1df3f8e3))
* **ci:** downgrade actions/checkout from v7.0.0 to v6.0.3 ([0dca9e8](https://github.com/djdembeck/m4b-merge/commit/0dca9e8e1b1cb517c9c280d7d19ab991a1b9899e))
* **ci:** restore missing uses directive for QEMU step ([eb17d81](https://github.com/djdembeck/m4b-merge/commit/eb17d81c9bf2d70554ce5e4cdbba5c30c1c10076))
* **ci:** restrict multi-platform Docker builds to version tags only ([4d1cff9](https://github.com/djdembeck/m4b-merge/commit/4d1cff9cea0c9c7a5d77c8237bf6843353da46a7))
* **ci:** update quinn-proto and Dockerfile for CI compliance ([1dbf00d](https://github.com/djdembeck/m4b-merge/commit/1dbf00d78210e8fe0f1b1019cb4581ed5382a175))
* **ci:** use single-platform build for PRs in docker-publish ([8af5240](https://github.com/djdembeck/m4b-merge/commit/8af52402f9e79fff483883fd6376f08486adfa7e))
* correct chpl parsing, CI pipelines, license, and code quality ([eb3b8ca](https://github.com/djdembeck/m4b-merge/commit/eb3b8caf80f3f04ff76e0b94595409ff99e199d7))
* **docker:** :ambulance: also chown /config ([56bbc4f](https://github.com/djdembeck/m4b-merge/commit/56bbc4f427cff6dd905508db6ac378031e4b8a0e))
* **docker:** :bug: better startup permissions management ([bd6c8a0](https://github.com/djdembeck/m4b-merge/commit/bd6c8a052fb0a25087595fa57482630c118bf2e9))
* **dry-run:** handle missing audio files gracefully in discovery ([4b2ca57](https://github.com/djdembeck/m4b-merge/commit/4b2ca572e50441bf5bfce6d46422930a74f47f02))
* file_title not found, replaced with title ([4924f61](https://github.com/djdembeck/m4b-merge/commit/4924f61aee96006cd5d96d301a8e6b55af7252fd))
* file_title not found, replaced with title ([325cb78](https://github.com/djdembeck/m4b-merge/commit/325cb782cd6b170262d707cfcfa5950104be1daa))
* **merge:** :ambulance: fix  inconsistent variable name ([296cee4](https://github.com/djdembeck/m4b-merge/commit/296cee458f4c36920f9f90410fd2330fed754cf5))
* **merge:** :ambulance: fix crash on single file in a folder ([a8fdc07](https://github.com/djdembeck/m4b-merge/commit/a8fdc07b8fdb364f851f019c5be8d1e728d72e96))
* **merge:** :bug: cleanup find_extension process ([5b6c0ff](https://github.com/djdembeck/m4b-merge/commit/5b6c0ffd2144360483bee79f935e68014c40a901))
* **merge:** :bug: don't create empty directory of file name ([5b9fb1f](https://github.com/djdembeck/m4b-merge/commit/5b9fb1fdb4144354c466224246c7c305578ea169))
* **merge:** :bug: fix asin validation before merge ([2baca86](https://github.com/djdembeck/m4b-merge/commit/2baca868a565118b97f045598269bff9b1871051))
* **merge:** :bug: fix error when no cover exists ([f5d3b23](https://github.com/djdembeck/m4b-merge/commit/f5d3b2340f43c6ef02ff7bc365a0955fb32ee904))
* **merge:** :bug: fix path comparison for junk dir ([eb41d8c](https://github.com/djdembeck/m4b-merge/commit/eb41d8cffa08b5cc07627f69770f744cf0f37f4f))
* **merge:** :bug: fix replace_tag replacing partial terms instead of full term ([88bafa8](https://github.com/djdembeck/m4b-merge/commit/88bafa8e1b94cde7d6f131aa4ff04f3935209a1f))
* **merge:** :bug: handle api having no author or narrators ([495c372](https://github.com/djdembeck/m4b-merge/commit/495c372b445c08ddf422888ac9a9ca3ef5c0d47b))
* **merge:** :bug: handle case where input has no `bit_rate` and/or `sample_rate` ([01988f0](https://github.com/djdembeck/m4b-merge/commit/01988f0697e1451e8c52efccee227f16669da825))
* **merge:** :bug: incorrect dict key ([e51bf14](https://github.com/djdembeck/m4b-merge/commit/e51bf146273a564470643a23f235b60b2672da42))
* **merge:** :bug: properly fix moving completed input files ([75c524b](https://github.com/djdembeck/m4b-merge/commit/75c524b05b35e1ab56d7269ad84bae89d0724067))
* **merge:** :bug: separate these into own functions so multi disc and single file both can pick up unknown extensions ([a61b2b5](https://github.com/djdembeck/m4b-merge/commit/a61b2b5d1e5a328e2c0bdc480aa276fb2d1e3c6d))
* **merge:** bug handle api having no author or narrators ([d97085d](https://github.com/djdembeck/m4b-merge/commit/d97085d6bc0a635f0f9bd7f3d86b4b98de56089b))
* replace deprecated Retry::spawn and update CI workflows ([9b800e6](https://github.com/djdembeck/m4b-merge/commit/9b800e636050eae8964d3f8b3523039c51395bb2))
* resolve CI failures (fmt, reqwest deps) ([fc30062](https://github.com/djdembeck/m4b-merge/commit/fc30062e412a43fa29a27e1efc940e7604da6d7d))
* restore dry-run guards, fix bitrate division, and re-add config validation ([87a6cf8](https://github.com/djdembeck/m4b-merge/commit/87a6cf867cbb133caa9f468e259b1374f06ea408))
* write temporary covers to `input_path` ([#104](https://github.com/djdembeck/m4b-merge/issues/104)) ([f6fa05e](https://github.com/djdembeck/m4b-merge/commit/f6fa05e22c75ca543401af4fdab81b0ac5b3bb26))


### Performance Improvements

* **docker:** cache rust deps via stub-source ([877f710](https://github.com/djdembeck/m4b-merge/commit/877f710c4c553081bdabc4c51b3e0cf2af33344c))


### Reverts

* move audiobookdb API migration to feature branch ([70c0514](https://github.com/djdembeck/m4b-merge/commit/70c0514b30868e7241664cd3e296f5df263b14d0))

## [Unreleased]

### Added
- Chapter embedding into M4B files using native mp4ameta library
- Chapters are now embedded directly into the M4B container, not just written to chapters.txt
- Full chapter metadata round-trip: extraction from source → embedding in output

### [0.5.3](https://github.com/djdembeck/m4b-merge/compare/v0.5.2...v0.5.3) (2024-08-07)

### Bug Fixes

  * If junk_dir is not set, do not perform post-process move ([4286a1c](https://github.com/djdembeck/m4b-merge/commit/4286a1ce7c50d56d5d9e22136cbdc292cd3d52e3))
  * Add --tmp-dir with os.pid to each m4b-tool invocation ([36689e8](https://github.com/djdembeck/m4b-merge/commit/36689e8f52ed7af7e3c70660501529d852dc482e))


### [0.5.2](https://github.com/djdembeck/m4b-merge/compare/v0.5.1...v0.5.2) (2023-04-27)


### Bug Fixes

* file_title not found, replaced with title ([3ec4d66](https://github.com/djdembeck/m4b-merge/commit/3ec4d661fd032836b374e277d2b947a170d16716))

### [0.5.1](https://github.com/djdembeck/m4b-merge/compare/v0.5.0...v0.5.1) (2023-02-24)


### Bug Fixes

* **merge:** :bug: incorrect dict key ([54d4a8b](https://github.com/djdembeck/m4b-merge/commit/54d4a8b259a0486ace02f69264aeacd7e224f26f))

## [0.5.0](https://github.com/djdembeck/m4b-merge/compare/v0.4.11...v0.5.0) (2023-02-24)


### Features

* **merge:** :sparkles: add support for `asin` as output path term ([87a3623](https://github.com/djdembeck/m4b-merge/commit/87a3623fd9799d5c7f30da34015b84b17eadb12d))

### [0.4.11](https://github.com/djdembeck/m4b-merge/compare/v0.4.10...v0.4.11) (2023-01-23)


### Bug Fixes

* write temporary covers to `input_path` ([#104](https://github.com/djdembeck/m4b-merge/issues/104)) ([7cfca92](https://github.com/djdembeck/m4b-merge/commit/7cfca92b61ad8f47a656418fb8385acc6625b0d9)), closes [#103](https://github.com/djdembeck/m4b-merge/issues/103)

### [0.4.10](https://github.com/djdembeck/m4b-merge/compare/v0.4.8...v0.4.10) (2022-09-21)


### Bug Fixes

* **merge:** :bug: properly fix moving completed input files ([f0f4ae9](https://github.com/djdembeck/m4b-merge/commit/f0f4ae9468796f13d6738cb4ba9592df9e858d74))

### [0.4.8](https://github.com/djdembeck/m4b-merge/compare/v0.4.7...v0.4.8) (2022-09-12)


### Features

* **merge:** :sparkles: use LOG_LEVEL from environment variable if available ([6779104](https://github.com/djdembeck/m4b-merge/commit/677910471c1ea88f272df29d1b5f0faf34e6b073))


### Bug Fixes

* **merge:** :ambulance: fix crash on single file in a folder ([a895b4d](https://github.com/djdembeck/m4b-merge/commit/a895b4de44f549068c4b010a3b4fb1a82d1750ad))
* **merge:** :bug: handle case where input has no `bit_rate` and/or `sample_rate` ([9e17fbd](https://github.com/djdembeck/m4b-merge/commit/9e17fbd7b58145461ca1cee422ab881e76415483))

### [0.4.7](https://github.com/djdembeck/m4b-merge/compare/v0.4.6...v0.4.7) (2022-02-28)


### Bug Fixes

* **docker:** :ambulance: also chown /config ([8e99393](https://github.com/djdembeck/m4b-merge/commit/8e993935e92cd2e49a10cd2abbec4cf394bbee83))
* **docker:** :bug: better startup permissions management ([3c4cef5](https://github.com/djdembeck/m4b-merge/commit/3c4cef567f185e2c690c043b2316c1e4439ed441))
* **merge:** :bug: cleanup find_extension process ([a37bfbe](https://github.com/djdembeck/m4b-merge/commit/a37bfbe96870774d35e3255813932f7ce2e7c518))
* **merge:** :bug: separate these into own functions so multi disc and single file both can pick up unknown extensions ([a8da6b5](https://github.com/djdembeck/m4b-merge/commit/a8da6b5ab3fe726057d4c9b18a7d486f5947990a))

### [0.4.6](https://github.com/djdembeck/m4b-merge/compare/v0.4.5...v0.4.6) (2022-02-07)

### [0.4.2](https://github.com/djdembeck/m4b-merge/compare/v0.4.1...v0.4.2) (2021-11-04)

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
