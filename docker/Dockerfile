FROM sandreas/ffmpeg:5.0.1-3 as ffmpeg
# There is an issue with the tone tagger
# https://github.com/sandreas/m4b-tool/issues/196
# FROM sandreas/tone:v0.1.0 as tone
FROM sandreas/mp4v2:2.1.1 as mp4v2
FROM sandreas/fdkaac:2.0.1 as fdkaac

FROM python:3.11-alpine3.15
ENV WORKDIR /mnt/
ENV M4BTOOL_TMP_DIR /tmp/m4b-tool/

RUN echo "---- INSTALL RUNTIME PACKAGES ----" && \
    apk add --no-cache --update --upgrade \
    # mp4v2: required libraries
    libstdc++ \
    # m4b-tool: php cli, required extensions and php settings
    php8-cli \
    php8-dom \
    php8-json \
    php8-xml \
    php8-mbstring \
    php8-phar \
    php8-tokenizer \
    php8-xmlwriter \
    php8-openssl \
    && echo "date.timezone = UTC" >> /etc/php8/php.ini \
    && ln -s /usr/bin/php8 /bin/php

# Add gosu for easy step-down from root
RUN apk add --no-cache --update --upgrade --repository=http://dl-cdn.alpinelinux.org/alpine/edge/testing \
    gosu

# Add ffmpeg, tone, mp4v2 and fdkaac
COPY --from=ffmpeg /usr/local/bin/ffmpeg /usr/local/bin/
COPY --from=ffmpeg /usr/local/bin/ffprobe /usr/local/bin/
# COPY --from=tone /usr/local/bin/tone /usr/local/bin/
COPY --from=mp4v2 /usr/local/bin/mp4* /usr/local/bin/
COPY --from=mp4v2 /usr/local/lib/libmp4v2* /usr/local/lib/
COPY --from=fdkaac /usr/local/bin/fdkaac /usr/local/bin/

ARG M4B_TOOL_DOWNLOAD_LINK="https://github.com/sandreas/m4b-tool/releases/latest/download/m4b-tool.tar.gz"

RUN echo "---- INSTALL M4B-TOOL ----" \
    && M4B_TOOL_PRE_RELEASE_LINK=$(wget -q -O - https://github.com/sandreas/m4b-tool/releases/tag/latest | grep -o 'M4B_TOOL_DOWNLOAD_LINK=[^ ]*' | head -1 | cut -d '=' -f 2) \
    && echo "!!! DOWNLOADING PRE_RELEASE ${M4B_TOOL_DOWNLOAD_LINK} !!!" && wget "${M4B_TOOL_PRE_RELEASE_LINK}" -O /tmp/m4b-tool.tar.gz \
    && tar xzf /tmp/m4b-tool.tar.gz -C /tmp/ && rm /tmp/m4b-tool.tar.gz \
    && mv /tmp/m4b-tool.phar /usr/local/bin/m4b-tool \
    && chmod +x /usr/local/bin/m4b-tool /usr/local/bin/m4b-tool

# set environment variables
ENV PYTHONDONTWRITEBYTECODE 1
ENV PYTHONUNBUFFERED 1

COPY . /src

# run this command to install all dependencies
RUN pip install --no-cache-dir --upgrade pip && \
    pip install --no-cache-dir -r /src/requirements.txt && \
    pip install --no-cache-dir /src && \
    rm -rf /src/build

# create the app user
ENV USER=abc
ENV UID=99

RUN \
    adduser \
    --disabled-password \
    --gecos "" \
    --home "$(pwd)" \
    --ingroup "users" \
    --no-create-home \
    --uid "$UID" \
    "$USER"


ENTRYPOINT ["/bin/sh", "/src/docker/entrypoint.sh"]
CMD m4b-merge
