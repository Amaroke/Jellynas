#!/usr/bin/env bash
set -uo pipefail

interval="${DEPLOY_INTERVAL:-300}"
watched="deployer/Dockerfile deployer/compose.yaml"

log() {
  printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*"
}

host_path() {
  local source drive
  source="$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/repo"}}{{.Source}}{{end}}{{end}}' "$(hostname)")"
  source="$(printf '%s' "$source" | tr '\\' /)"
  case "$source" in
    [A-Za-z]:/*)
      drive="$(printf '%s' "${source%%:*}" | tr '[:upper:]' '[:lower:]')"
      source="/run/desktop/mnt/host/$drive${source#?:}"
      ;;
  esac
  printf '%s' "$source"
}

enter_repo() {
  local path
  path="$(host_path)"
  if [ -z "$path" ]; then
    log "cannot find the host path of /repo"
    return 1
  fi
  if [ "$path" != /repo ] && [ ! -e "$path" ]; then
    mkdir -p "$(dirname "$path")"
    ln -s /repo "$path"
  fi
  cd "$path"
}

recreate_self() {
  docker run -d --rm \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -v "$PWD:/repo" \
    jellynas-deployer bash /repo/deployer/entrypoint.sh recreate > /dev/null
}

git config --global safe.directory '*'

if ! enter_repo; then
  sleep "$interval"
  exit 1
fi

if [ "${1:-}" = recreate ]; then
  docker compose -f deployer/compose.yaml up -d --build
  exit
fi

state="$(git rev-parse --git-path jellynas-deployed)"
before="$(cat "$state" 2>/dev/null)"
if bash scripts/deploy.sh; then
  after="$(cat "$state" 2>/dev/null)"
  if [ -n "$before" ] && [ "$before" != "$after" ] && ! git diff --quiet "$before" "$after" -- $watched; then
    log "the deployer changed, recreating it"
    recreate_self
  fi
fi

sleep "$interval"
exec bash /repo/deployer/entrypoint.sh
