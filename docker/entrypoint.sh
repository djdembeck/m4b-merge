#!/bin/sh

chown -R worker:worker /input /output
exec runuser -u worker "$@"