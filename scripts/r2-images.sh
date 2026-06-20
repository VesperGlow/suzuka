#!/usr/bin/env bash

set -euo pipefail

readonly CONFIG_FILE="${HOME}/.config/r2-blog.env"

usage() {
    printf 'Usage: %s <prefix>\n' "${0##*/}" >&2
}

if [[ $# -ne 1 ]]; then
    usage
    exit 2
fi

if [[ ! -r "${CONFIG_FILE}" ]]; then
    printf 'Error: R2 configuration is not readable: %s\n' "${CONFIG_FILE}" >&2
    exit 1
fi

if ! command -v aws >/dev/null 2>&1; then
    printf 'Error: AWS CLI is required but was not found.\n' >&2
    exit 1
fi

set -a
# shellcheck disable=SC1090
source "${CONFIG_FILE}"
set +a

for variable in R2_ENDPOINT R2_BUCKET; do
    if [[ -z "${!variable:-}" ]]; then
        printf 'Error: %s is not set in %s\n' "${variable}" "${CONFIG_FILE}" >&2
        exit 1
    fi
done

aws s3api list-objects-v2 \
    --endpoint-url "${R2_ENDPOINT}" \
    --bucket "${R2_BUCKET}" \
    --prefix "$1" \
    --query 'Contents[?Size > `0`].{Key:Key,Size:Size}' \
    --output table
