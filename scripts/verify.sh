#!/usr/bin/env bash
set -uo pipefail

export MSYS_NO_PATHCONV=1

cd "$(dirname "$0")/.." || exit 1

failures=0
health_timeout="${VERIFY_HEALTH_TIMEOUT:-120}"

pass() {
  printf 'PASS [%s] %s\n' "$1" "$2"
}

fail() {
  printf 'FAIL [%s] %s\n' "$1" "$2"
  failures=$((failures + 1))
}

container_state() {
  local id
  id="$(docker compose ps -a -q "$1" 2>/dev/null | head -n 1)"
  if [ -z "$id" ]; then
    echo "missing"
    return
  fi
  docker inspect -f '{{.State.Status}}{{if .State.Health}}/{{.State.Health.Status}}{{end}}' "$id"
}

check_compose_renders() {
  if docker compose --env-file .env.example config --quiet; then
    pass compose "config renders with .env.example"
  else
    fail compose "config does not render with .env.example"
  fi
}

check_env_filled() {
  local name value missing=""
  if [ ! -f .env ]; then
    fail env ".env is missing, copy .env.example and fill it in"
    return
  fi
  for name in $(grep -oE '^[A-Z_]+=' .env.example | tr -d '='); do
    value="$(grep -E "^$name=" .env | tail -n 1 | cut -d= -f2- | tr -d '\r')"
    [ -n "$value" ] || missing="$missing $name"
  done
  if [ -n "$missing" ]; then
    fail env "empty or missing in .env:$missing"
  else
    pass env "every variable of .env.example is set in .env"
  fi
}

check_containers_up() {
  local service services state deadline
  if ! services="$(docker compose config --services)" || [ -z "$services" ]; then
    fail compose "cannot list services from the config"
    return
  fi
  deadline=$((SECONDS + health_timeout))
  for service in $services; do
    state="$(container_state "$service")"
    while { [ "$state" = "running/starting" ] || [ "$state" = "created" ] || [ "${state%%/*}" = "restarting" ]; } && [ "$SECONDS" -lt "$deadline" ]; do
      sleep 5
      state="$(container_state "$service")"
    done
    case "$state" in
      running/healthy | running)
        pass "$service" "container is $state"
        ;;
      *)
        fail "$service" "container is $state, expected healthy or running"
        ;;
    esac
  done
}

check_read_only() {
  local service="$1" path="$2" probe
  probe="$path/.verify-write-probe"
  case "$(container_state "$service")" in
    running*) ;;
    *)
      fail "$service" "is not running, cannot check that $path is read-only"
      return
      ;;
  esac
  if ! docker compose exec -T "$service" test -d "$path" 2>/dev/null; then
    fail "$service" "cannot see $path, cannot check that it is read-only"
    return
  fi
  if docker compose exec -T "$service" sh -c "touch '$probe'" 2>/dev/null; then
    docker compose exec -T "$service" rm -f "$probe" >/dev/null 2>&1
    fail "$service" "can write to $path"
  else
    pass "$service" "cannot write to $path"
  fi
}

check_compose_renders
check_env_filled
check_containers_up
check_read_only jellyfin /data/media

if [ "$failures" -gt 0 ]; then
  printf '\n%d check(s) failed\n' "$failures"
  exit 1
fi
printf '\nAll checks passed\n'
