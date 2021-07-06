from distutils.core import setup

setup(
	name='M4bMerge',
	version='0.3.2',
	packages=['m4bmerge', 'm4bmerge.test'],
	url='https://github.com/djdembeck/m4b-merge',
	license='LICENSE',
	description='A tool to standardize audiobook files into single m4b files with clean metadata based on Audible and chapters.',
	# long_description=open('README.txt').read(),
	install_requires=[
		"html2text>=2020.1.16"
		"pydub>=0.25.1"
		"requests>=2.24.0"
		"pathvalidate>=2.4.1"
		"audible>=0.5.4"
	],
)