# Must be 3.10 as compile fails on 3.11+
FROM alpine:3.10 AS ffbase

RUN buildDeps="autoconf \
  automake \
  bash \
  binutils \
  build-base \
  bzip2 \
  cmake \
  curl \
  coreutils \
  diffutils \
  expat-dev \
  file \
  findutils \
  g++ \
  gperf \
  libarchive-tools \
  libgomp \
  libtool \
  nasm \
  python3 \
  openssl \
  openssl-dev \
  tar \
  util-linux-dev \
  yasm \
  zlib-dev" && \
  apk add --no-cache ${buildDeps}

FROM ffbase AS ffbuild

WORKDIR /tmp/workdir

ARG PKG_CONFIG_PATH=/opt/ffmpeg/lib/pkgconfig
ARG LD_LIBRARY_PATH=/opt/ffmpeg/lib
ARG PREFIX=/opt/ffmpeg
ARG MAKEFLAGS="-j$(nproc)"

ENV FFMPEG_VERSION=snapshot \
  FREETYPE_VERSION=2.10.1 \
  LAME_VERSION=3.100 \
  OPUS_VERSION=1.3.1 \
  X264_VERSION=x264-master \
  X265_VERSION=3.4 \
  ZIMG_VERSION=2.9.3 \
  SRC=/usr/local

ARG FREETYPE_SHA256SUM="3a60d391fd579440561bf0e7f31af2222bc610ad6ce4d9d7bd2165bca8669110 freetype-${FREETYPE_VERSION}.tar.gz"
ARG OPUS_SHA256SUM="65b58e1e25b2a114157014736a3d9dfeaad8d41be1c8179866f144a2fb44ff9d opus-${OPUS_VERSION}.tar.gz"

### fdk-aac https://github.com/mstorsjo/fdk-aac
RUN \
        DIR=/tmp/fdk-aac && \
        mkdir -p ${DIR} && \
        cd ${DIR} && \
        curl -sL https://github.com/mstorsjo/fdk-aac/archive/master.zip -o fdk-aac-master.zip && \
        bsdtar --strip-components=1 -xf fdk-aac-master.zip && \
        rm fdk-aac-master.zip && \
        autoreconf -fiv && \
        ./configure --prefix="${PREFIX}" --disable-shared --datadir="${DIR}" && \
        make && \
        make install

## x264 http://www.videolan.org/developers/x264.html
RUN \
        DIR=/tmp/x264 && \
        mkdir -p ${DIR} && \
        cd ${DIR} && \
        curl -sL https://code.videolan.org/videolan/x264/-/archive/master/${X264_VERSION}.tar.bz2 | \
        tar -jx --strip-components=1 && \
        ./configure --prefix="${PREFIX}" --enable-static --enable-pic --disable-cli && \
        make && \
        make install

### x265 http://x265.org/
RUN \
        DIR=/tmp/x265 && \
        mkdir -p ${DIR} && \
        cd ${DIR} && \
        curl -sL http://anduin.linuxfromscratch.org/BLFS/x265/x265_${X265_VERSION}.tar.gz  | \
        tar -zx && \
        cd x265_${X265_VERSION}/build/linux && \
        find . -mindepth 1 ! -name 'make-Makefiles.bash' -and ! -name 'multilib.sh' -exec rm -r {} + && \
        cmake -G "Unix Makefiles" -DCMAKE_INSTALL_PREFIX="$PREFIX" -DENABLE_SHARED:BOOL=OFF -DSTATIC_LINK_CRT:BOOL=ON -DENABLE_CLI:BOOL=OFF ../../source && \
        sed -i 's/-lgcc_s/-lgcc_eh/g' x265.pc && \
        make && \
        make install

### libopus https://www.opus-codec.org/
RUN \
        DIR=/tmp/opus && \
        mkdir -p ${DIR} && \
        cd ${DIR} && \
        curl -sLO https://archive.mozilla.org/pub/opus/opus-${OPUS_VERSION}.tar.gz && \
        echo ${OPUS_SHA256SUM} | sha256sum --check && \
        tar -zx --strip-components=1 -f opus-${OPUS_VERSION}.tar.gz && \
        autoreconf -fiv && \
        ./configure --prefix="${PREFIX}" --disable-shared && \
        make && \
        make install
        
### libmp3lame http://lame.sourceforge.net/
RUN \
        DIR=/tmp/lame && \
        mkdir -p ${DIR} && \
        cd ${DIR} && \
        curl -sL https://versaweb.dl.sourceforge.net/project/lame/lame/$(echo ${LAME_VERSION} | sed -e 's/[^0-9]*\([0-9]*\)[.]\([0-9]*\)[.]\([0-9]*\)\([0-9A-Za-z-]*\)/\1.\2/')/lame-${LAME_VERSION}.tar.gz | \
        tar -zx --strip-components=1 && \
        ./configure --prefix="${PREFIX}" --bindir="${PREFIX}/bin" --disable-shared --enable-nasm --enable-pic --disable-frontend && \
        make && \
        make install

## freetype https://www.freetype.org/
RUN  \
        DIR=/tmp/freetype && \
        mkdir -p ${DIR} && \
        cd ${DIR} && \
        curl -sLO https://download.savannah.gnu.org/releases/freetype/freetype-${FREETYPE_VERSION}.tar.gz && \
        echo ${FREETYPE_SHA256SUM} | sha256sum --check && \
        tar -zx --strip-components=1 -f freetype-${FREETYPE_VERSION}.tar.gz && \
        ./configure --prefix="${PREFIX}" --enable-static --disable-shared && \
        make && \
        make install

## Zimg
RUN  \
        DIR=/tmp/zimg && \
        mkdir -p ${DIR} && \
        cd ${DIR} && \
        curl -sLO https://github.com/sekrit-twc/zimg/archive/release-${ZIMG_VERSION}.tar.gz &&\
        tar -zx --strip-components=1 -f release-${ZIMG_VERSION}.tar.gz && \
        ./autogen.sh && \
        ./configure --enable-static -prefix="${PREFIX}" --disable-shared && \
        make && \
        make install

## ffmpeg https://ffmpeg.org/
RUN  \
        DIR=/tmp/ffmpeg && mkdir -p ${DIR} && cd ${DIR} && \
        curl -LO https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.bz2 && \
        tar -jx --strip-components=1 -f ffmpeg-${FFMPEG_VERSION}.tar.bz2

RUN \
        DIR=/tmp/ffmpeg && mkdir -p ${DIR} && cd ${DIR} && \
        ./configure \
        --enable-ffplay \
        --enable-gpl \
        --enable-version3 \
        --enable-libfdk-aac \
        --enable-libfreetype \
        --enable-libmp3lame \
        --enable-libopus \
        --enable-libx264 \
        --enable-libx265 \
        --enable-libzimg \
        --enable-nonfree \
        --enable-openssl \
        --pkg-config-flags="--static" \
        --extra-cflags="-I$PREFIX/include" \
        --extra-ldflags="-L$PREFIX/lib" \
        --extra-libs="-lpthread -lm -lz" \
        --extra-ldexeflags="-static" \
        --prefix="${PREFIX}" && \
        make && \
        make install

FROM alpine:3.12 AS base

RUN buildDeps="autoconf \
  automake \
  boost-dev \
  build-base \
  libtool \
  tar \
  unzip \
  wget" && \
  apk add --no-cache ${buildDeps}

FROM base AS build

WORKDIR /tmp/workdir

# Compile Sandreas mp4v2
RUN \
  wget https://github.com/sandreas/mp4v2/archive/master.zip && \
  unzip master.zip && \
  cd mp4v2-master && \
  ./configure && \
  make -j4 && \
  make install && make distclean

ARG FDK_AAC_VERSION=2.0.1
ARG FDK_AAC_URL="https://github.com/mstorsjo/fdk-aac/archive/v$FDK_AAC_VERSION.tar.gz"

# Compile fdkaac
RUN \
  wget -O fdk-aac.tar.gz "$FDK_AAC_URL" && \
  tar xfz fdk-aac.tar.gz && \
  cd fdk-aac-* && ./autogen.sh && ./configure --enable-static --disable-shared && \
  make -j$(nproc) install && \
  cd /tmp/workdir && \
  wget https://github.com/nu774/fdkaac/archive/1.0.0.tar.gz && \
  tar xzf 1.0.0.tar.gz && \
  cd fdkaac-1.0.0 && \
  autoreconf -i && ./configure --enable-static --disable-shared && \
  make -j4 && make install

FROM alpine:3.12

WORKDIR /tmp/workdir

RUN apk add --no-cache --update \
  bash \
  curl \
  grep \
  libstdc++ \
  mutagen \
  php7-cli \
  php7-dom \
  php7-json \
  php7-xml \
  php7-mbstring \
  php7-phar \
  php7-tokenizer \
  php7-xmlwriter \
  php7-openssl \
  pv \
  tar \
  wget && \
  echo "date.timezone = UTC" >> /etc/php7/php.ini

RUN apk add --no-cache --repository http://dl-cdn.alpinelinux.org/alpine/edge/community/ --allow-untrusted gnu-libiconv

ENV LD_PRELOAD /usr/lib/preloadable_libiconv.so php

RUN \
  M4B_TOOL_PRE_RELEASE_LINK="$(wget -q -O - https://github.com/sandreas/m4b-tool/releases/tag/latest | grep M4B_TOOL_DOWNLOAD_LINK | cut -d '=' -f 2 | cut -d ' ' -f 1)" && \
  wget "$M4B_TOOL_PRE_RELEASE_LINK" -O /tmp/m4b-tool.tar.gz && \
  tar -xf /tmp/m4b-tool.tar.gz -C /tmp && \
  rm /tmp/m4b-tool.tar.gz && \
  mv /tmp/m4b-tool.phar /usr/local/bin/m4b-tool && \
  chmod +x /usr/local/bin/m4b-tool

RUN apk del \
  tar \
  wget
  
RUN addgroup -S 100 && \
  adduser --home /app --shell /bin/bash --uid 99 -S -G 100 abc

COPY --chown=99:100 ./m4b-merge.sh /app/m4b-merge.sh

COPY --from=ffbuild /opt/ffmpeg/bin/ffmpeg /usr/local/bin
COPY --from=ffbuild /opt/ffmpeg/bin/ffprobe /usr/local/bin

COPY --from=build /usr/local/bin/mp4* /usr/local/bin/fdkaac /usr/local/bin/
COPY --from=build /usr/local/lib/libmp4v2* /usr/local/lib/

RUN printf '#!/bin/bash \n /app/m4b-merge.sh "$@"' > /usr/bin/m4b-merge && \
    chmod +x /usr/bin/m4b-merge

USER 99:100

WORKDIR /app

CMD tail -f /dev/null
