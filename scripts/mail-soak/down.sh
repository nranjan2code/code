#!/bin/sh
docker rm -f mail-soak >/dev/null 2>&1 || true
docker volume rm -f mail-soak-data >/dev/null 2>&1 || true
echo down
