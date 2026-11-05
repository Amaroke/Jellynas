#!/usr/bin/env bash
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

branch="${DEPLOY_BRANCH:-main}"
remote="${DEPLOY_REMOTE:-origin}"
state="$(git rev-parse --git-path jellynas-deployed)"

log() {
  printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*"
}

folder_services() {
  awk '
    /^services:/ { inside = 1; next }
    /^[^[:space:]#]/ { inside = 0 }
    inside && /^  [A-Za-z0-9._-]+:[[:space:]]*$/ { sub(/^  /, ""); sub(/:.*/, ""); print }
  ' "$1/compose.yaml"
}

changed_folders() {
  local path folder
  while IFS= read -r path; do
    folder="${path%%/*}"
    [ "$folder" != "$path" ] || continue
    [ "${path#*/}" != compose.yaml ] || continue
    [ -f "$folder/compose.yaml" ] || continue
    printf '%s\n' "$folder"
  done | sort -u
}

services_to_restart() {
  local folders folder service enabled
  folders="$(changed_folders)"
  [ -n "$folders" ] || return 0
  enabled=" $(docker compose config --services | tr '\n' ' ') "
  for folder in $folders; do
    for service in $(folder_services "$folder"); do
      case "$enabled" in
        *" $service "*) printf '%s\n' "$service" ;;
      esac
    done
  done
}


main() {
  local current before target deployed base changed restart
  current="$(git rev-parse --abbrev-ref HEAD)"
  if [ "$current" != "$branch" ]; then
    log "checked out on $current instead of $branch, nothing deployed"
    return 1
  fi

  if ! git diff --quiet HEAD --; then
    log "tracked files changed locally, commit them through a pull request or discard them, nothing deployed"
    return 1
  fi

  if ! git fetch --quiet "$remote" "$branch"; then
    log "cannot fetch $branch from $remote, nothing deployed"
    return 1
  fi

  before="$(git rev-parse HEAD)"
  if ! git merge --quiet --ff-only FETCH_HEAD; then
    log "$branch cannot fast-forward to $remote/$branch, see the git error above, nothing deployed"
    return 1
  fi

  target="$(git rev-parse HEAD)"
  deployed="$(cat "$state" 2>/dev/null)"
  [ "$deployed" != "$target" ] || return 0

  base="$before"
  if [ -n "$deployed" ] && git cat-file -e "$deployed^{commit}" 2>/dev/null; then
    base="$deployed"
  fi
  changed="$(git diff --name-only "$base" "$target")"

  log "deploying $(git log -1 --format='%h %s' "$target")"

  if ! docker compose up -d --build --remove-orphans; then
    log "docker compose up failed, the next run retries"
    return 1
  fi

  if [ -n "$changed" ]; then
    restart="$(printf '%s\n' "$changed" | services_to_restart | tr '\n' ' ')"
    restart="${restart% }"
    if [ -n "$restart" ]; then
      log "restarting $restart to load their changed files"
      if ! docker compose restart $restart; then
        log "docker compose restart failed, the next run retries"
        return 1
      fi
    fi
  fi

  printf '%s\n' "$target" > "$state"
  log "deployed $(git rev-parse --short "$target")"
}

main "$@"
exit
