#!/bin/sh
set -eu

usage() {
  echo "Usage: check.sh radarr|sonarr|lazylibrarian" >&2
  echo "       check.sh api radarr|sonarr|prowlarr|lazylibrarian METHOD PATH [JSON]" >&2
  exit 2
}

root=$(cd "$(dirname "$0")/.." && pwd)
env_file=${CHECK_ENV_FILE:-$root/.env}

clone() {
  git -c safe.directory="$root" -C "$root" "$@"
}

if [ -z "${CHECK_ANY_BRANCH:-}" ]; then
  if [ "$(clone branch --show-current)" != main ] || [ -n "$(clone status --porcelain)" ]; then
    echo "The Clone must be on main without local changes" >&2
    exit 1
  fi
fi

compose() {
  docker compose -f "$root/services/check/compose.yaml" --env-file "$env_file" "$@"
}

facts() {
  for name in "$@"; do
    state=$(docker inspect -f '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}' "$name" 2>/dev/null || echo missing)
    echo "state|$name|$state"
  done
  docker inspect -f '{{range .Mounts}}{{println .Destination}}{{end}}' lazylibrarian 2>/dev/null |
    sed '/^$/d; s/^/mount|lazylibrarian|/'
  for name in "$@"; do
    docker logs -t --since 24h "$name" 2>&1 | sed "s/^/log|$name|/"
  done
}

case "${1:-}" in
  radarr | sonarr | lazylibrarian)
    compose build --quiet check
    facts "$1" prowlarr gluetun qbittorrent ratio | compose run --rm -T check "$1"
    ;;
  api)
    shift
    compose build --quiet check
    compose run --rm -T check api "$@"
    ;;
  *)
    usage
    ;;
esac
