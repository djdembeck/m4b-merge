# Forked from https://github.com/wader/static-ffmpeg/blob/master/Dockerfile
FROM alpine:3.14.2 AS ffbuild

ARG FFMPEG_VERSION=4.4.1
ARG FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.bz2"
ARG FFMPEG_SHA256=8fc9f20ac5ed95115a9e285647add0eedd5cc1a98a039ada14c132452f98ac42

ARG MP3LAME_VERSION=3.100
ARG MP3LAME_URL="https://sourceforge.net/projects/lame/files/lame/$MP3LAME_VERSION/lame-$MP3LAME_VERSION.tar.gz/download"
ARG MP3LAME_SHA256=ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e

ARG FDK_AAC_VERSION=2.0.2
ARG FDK_AAC_URL="https://github.com/mstorsjo/fdk-aac/archive/v$FDK_AAC_VERSION.tar.gz"
ARG FDK_AAC_SHA256=7812b4f0cf66acda0d0fe4302545339517e702af7674dd04e5fe22a5ade16a90

ARG OPUS_VERSION=1.3.1
ARG OPUS_URL="https://archive.mozilla.org/pub/opus/opus-$OPUS_VERSION.tar.gz"
ARG OPUS_SHA256=65b58e1e25b2a114157014736a3d9dfeaad8d41be1c8179866f144a2fb44ff9d

ARG CFLAGS="-O3 -static-libgcc -fno-strict-overflow -fstack-protector-all -fPIE"
ARG CXXFLAGS="-O3 -static-libgcc -fno-strict-overflow -fstack-protector-all -fPIE"
ARG LDFLAGS="-Wl,-z,relro,-z,now"

# Can probably clean these out
RUN apk add --no-cache \
    coreutils \
    rust \
    cargo \
    openssl openssl-dev openssl-libs-static \
    ca-certificates \
    bash \
    tar \
    build-base \
    autoconf \
    automake \
    libtool \
    diffutils \
    cmake \
    meson \
    ninja \
    git \
    yasm \
    nasm \
    texinfo \
    jq \
    zlib zlib-dev zlib-static \
    libbz2 bzip2-dev bzip2-static \
    libxml2 libxml2-dev \
    expat expat-dev expat-static \
    fontconfig fontconfig-dev fontconfig-static \
    freetype freetype-dev freetype-static \
    graphite2-static \
    glib-static \
    tiff tiff-dev \
    libjpeg-turbo libjpeg-turbo-dev \
    libpng-dev libpng-static \
    giflib giflib-dev \
    harfbuzz harfbuzz-dev harfbuzz-static \
    fribidi fribidi-dev fribidi-static \
    brotli brotli-dev brotli-static \
    soxr soxr-dev soxr-static \
    tcl \
    numactl numactl-dev \
    cunit cunit-dev \
    xxd

RUN \
    OPENSSL_VERSION=$(pkg-config --modversion openssl) \
    LIBXML2_VERSION=$(pkg-config --modversion libxml-2.0) \
    EXPAT_VERSION=$(pkg-config --modversion expat) \
    FREETYPE_VERSION=$(pkg-config --modversion freetype2)  \
    FONTCONFIG_VERSION=$(pkg-config --modversion fontconfig)  \
    FRIBIDI_VERSION=$(pkg-config --modversion fribidi)  \
    SOXR_VERSION=$(pkg-config --modversion soxr) \
    jq -n \
    '{ \
    ffmpeg: env.FFMPEG_VERSION, \
    openssl: env.OPENSSL_VERSION, \
    libxml2: env.LIBXML2_VERSION, \
    expat: env.EXPAT_VERSION, \
    libmp3lame: env.MP3LAME_VERSION, \
    "libfdk-aac": env.FDK_AAC_VERSION, \
    libopus: env.OPUS_VERSION, \
    }' > /versions.json

RUN \
    wget -O lame.tar.gz "$MP3LAME_URL" && \
    echo "$MP3LAME_SHA256  lame.tar.gz" | sha256sum --status -c - && \
    tar xf lame.tar.gz && \
    cd lame-* && ./configure --disable-shared --enable-static --enable-nasm --disable-gtktest --disable-cpml --disable-frontend && \
    make -j$(nproc) install

RUN \
    wget -O fdk-aac.tar.gz "$FDK_AAC_URL" && \
    echo "$FDK_AAC_SHA256  fdk-aac.tar.gz" | sha256sum --status -c - && \
    tar xf fdk-aac.tar.gz && \
    cd fdk-aac-* && ./autogen.sh && ./configure --disable-shared --enable-static && \
    make -j$(nproc) install

RUN \
    wget -O opus.tar.gz "$OPUS_URL" && \
    echo "$OPUS_SHA256  opus.tar.gz" | sha256sum --status -c - && \
    tar xf opus.tar.gz && \
    cd opus-* && ./configure --disable-shared --enable-static --disable-extra-programs && \
    make -j$(nproc) install

RUN \
    wget -O ffmpeg.tar.bz2 "$FFMPEG_URL" && \
    echo "$FFMPEG_SHA256  ffmpeg.tar.bz2" | sha256sum --status -c - && \
    tar xf ffmpeg.tar.bz2 && \
    cd ffmpeg-* && \
    sed -i 's/add_ldexeflags -fPIE -pie/add_ldexeflags -fPIE -static-pie/' configure && \
    ./configure \
    --pkg-config-flags="--static" \
    --extra-cflags="-fopenmp" \
    --extra-ldflags="-fopenmp" \
    --extra-libs="-lstdc++" \
    --toolchain=hardened \
    --disable-debug \
    --disable-shared \
    --disable-ffplay \
    --enable-static \
    --enable-gpl \
    --enable-gray \
    --enable-nonfree \
    --enable-openssl \
    --enable-iconv \
    --enable-libxml2 \
    --enable-libmp3lame \
    --enable-libfdk-aac \
    || (cat ffbuild/config.log ; false) \
    && make -j$(nproc) install tools/qt-faststart \
    && cp tools/qt-faststart /usr/local/bin

# base image
FROM python:3.9

RUN \
    touch /etc/apt/sources.list.d/contrib.list && \
    echo "deb http://ftp.us.debian.org/debian buster main contrib non-free" >> /etc/apt/sources.list.d/contrib.list 

# Get dependencies for m4b-tool/ffmpeg
RUN	apt-get update && \
    apt-get install --no-install-recommends -y \
    fdkaac \
    php-cli \
    php-common \
    php-intl \
    php-mbstring \
    php-xml \
    wget && \
    M4B_TOOL_PRE_RELEASE_LINK="$(wget -nv -O - https://github.com/sandreas/m4b-tool/releases/tag/latest | grep -o 'M4B_TOOL_DOWNLOAD_LINK=[^ ]*' | head -1 | cut -d '=' -f 2)" && \
    wget --progress=dot:giga "$M4B_TOOL_PRE_RELEASE_LINK" -O /tmp/m4b-tool.tar.gz && \
    tar -xf /tmp/m4b-tool.tar.gz -C /tmp && \
    rm /tmp/m4b-tool.tar.gz && \
    mv /tmp/m4b-tool.phar /usr/local/bin/m4b-tool && \
    chmod +x /usr/local/bin/m4b-tool && \
    wget --progress=dot:giga http://archive.ubuntu.com/ubuntu/pool/universe/m/mp4v2/libmp4v2-2_2.0.0~dfsg0-6_amd64.deb && \
    wget --progress=dot:giga http://archive.ubuntu.com/ubuntu/pool/universe/m/mp4v2/mp4v2-utils_2.0.0~dfsg0-6_amd64.deb && \
    dpkg -i libmp4v2-2_2.0.0~dfsg0-6_amd64.deb && \
    dpkg -i mp4v2-utils_2.0.0~dfsg0-6_amd64.deb && \
    rm ./*.deb && \
    rm -rf /var/lib/apt/lists/* && \
    apt-get remove -y wget

# set environment variables
ENV PYTHONDONTWRITEBYTECODE 1
ENV PYTHONUNBUFFERED 1

COPY . /src

# run this command to install all dependencies
RUN pip install --no-cache-dir --upgrade pip && \
    pip install --no-cache-dir -r /src/requirements.txt \
    pip install --no-cache-dir /src && \
    rm -rf /src/build

COPY --from=ffbuild /usr/local/bin/ffmpeg /usr/bin
COPY --from=ffbuild /usr/local/bin/ffprobe /usr/bin

ENTRYPOINT ["/bin/sh", "/src/docker/entrypoint.sh"]
CMD m4b-merge