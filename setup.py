import setuptools

with open("README.md", "r", encoding="utf-8") as fh:
    long_description = fh.read()

setuptools.setup(
    name='m4b-merge',
    version='0.3.5.1',
    url='https://github.com/djdembeck/m4b-merge',
    description=(
                "A tool to standardize audiobook files"
                " into single m4b files with clean metadata"
                " based on Audible and chapters."),
    long_description=long_description,
    long_description_content_type="text/markdown",
    author='David Dembeck',
    author_email='dembeck.david@protonmail.com',
    project_urls={
        "Bug Tracker": "https://github.com/djdembeck/m4b-merge/issues",
    },
    classifiers=[
        "Programming Language :: Python :: 3.7",
        "Programming Language :: Python :: 3.8",
        "Programming Language :: Python :: 3.9",
        "License :: OSI Approved :: GNU General Public License v3 (GPLv3)",
        "Operating System :: OS Independent",
    ],
    install_requires=[
        'appdirs>=1.4.4',
        'audible>=0.5.4',
        'html2text>=2020.1.16',
        'pathvalidate>=2.4.1',
        'pydub>=0.25.1',
        'requests>=2.24.0',
    ],
    package_dir={"": "src"},
    packages=["m4b_merge"],
    python_requires=">=3.6",
    entry_points={'console_scripts': ['m4b-merge = m4b_merge.__main__:main']}
)
