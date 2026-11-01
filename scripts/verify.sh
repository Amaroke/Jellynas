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

env_value() {
  local value ref
  value="$(grep -E "^$1=" .env 2>/dev/null | tail -n 1 | cut -d= -f2- | tr -d '\r' | sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'$/\1/")"
  while [[ "$value" =~ \$\{([A-Z_]+)\} ]]; do
    ref="${BASH_REMATCH[1]}"
    [ "$ref" != "$1" ] || break
    value="${value//\$\{$ref\}/$(env_value "$ref")}"
  done
  printf '%s' "$value"
}

http_code() {
  curl --silent --output /dev/null --write-out '%{http_code}' --max-time 10 "$@"
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
    grep -rqF "\${$name:-" --include=compose.yaml . && continue
    value="$(env_value "$name")"
    [ -n "$value" ] || missing="$missing $name"
  done
  if [ -n "$missing" ]; then
    fail env "empty or missing in .env:$missing"
  else
    pass env "every required variable of .env.example is set in .env"
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

check_internet_ports() {
  local service id ports exposed=""
  for service in $(docker compose config --services 2>/dev/null); do
    [ "$service" = caddy ] && continue
    id="$(docker compose ps -q "$service" 2>/dev/null | head -n 1)"
    [ -n "$id" ] || continue
    ports="$(docker inspect -f '{{range $p, $b := .HostConfig.PortBindings}}{{range $b}} {{.HostPort}}{{end}}{{end}}' "$id")"
    case "$ports " in
      *" $https_port "*) exposed="$exposed $service" ;;
    esac
  done
  if [ -n "$exposed" ]; then
    fail compose "only caddy may publish port $https_port, also published by:$exposed"
  else
    pass compose "only caddy publishes port $https_port"
  fi
}

caddy_running() {
  case "$(container_state caddy)" in
    running*) return 0 ;;
  esac
  fail caddy "is not running, cannot check $1"
  return 1
}

check_public_hostname() {
  local host="$1" code deadline
  [ -n "$host" ] || return
  caddy_running "https://$host" || return
  deadline=$((SECONDS + health_timeout))
  code="$(http_code --resolve "$host:$https_port:127.0.0.1" "https://$host:$https_port/")"
  while [ "$code" = 000 ] && [ "$SECONDS" -lt "$deadline" ]; do
    sleep 5
    code="$(http_code --resolve "$host:$https_port:127.0.0.1" "https://$host:$https_port/")"
  done
  case "$code" in
    2?? | 3??)
      pass caddy "https://$host answers $code with a valid certificate"
      ;;
    *)
      fail caddy "https://$host does not answer with a valid certificate (HTTP $code)"
      ;;
  esac
}

check_unknown_host_refused() {
  local known="$1" unknown="unknown.invalid" code
  caddy_running "that unknown hosts are refused" || return
  code="$(http_code --insecure --resolve "$unknown:$https_port:127.0.0.1" "https://$unknown:$https_port/")"
  if [ "$code" = 000 ]; then
    pass caddy "refuses an unknown host over HTTPS"
  else
    fail caddy "answers $code to an unknown host over HTTPS"
  fi
  [ -n "$known" ] || return
  code="$(http_code --insecure --resolve "$known:$https_port:127.0.0.1" --header "Host: $unknown" "https://$known:$https_port/")"
  if [ "$code" = 000 ]; then
    pass caddy "refuses an unknown host sent over a known TLS name"
  else
    fail caddy "answers $code to an unknown host sent over a known TLS name"
  fi
}

jellyfin_hostname="$(env_value JELLYFIN_HOSTNAME)"
https_port="$(env_value CADDY_HTTPS_PORT)"
https_port="${https_port:-443}"
public_port=""
[ "$https_port" = 443 ] || public_port=":$https_port"

check_compose_renders
check_env_filled
check_containers_up
check_read_only jellyfin /data/media
check_internet_ports
check_public_hostname "$jellyfin_hostname"
check_unknown_host_refused "$jellyfin_hostname"

if [ -n "$jellyfin_hostname" ]; then
  printf '\nOn a phone on 4G with wifi off, open each address and check that it loads:\n'
  printf '  https://%s%s\n' "$jellyfin_hostname" "$public_port"
fi

if [ "$failures" -gt 0 ]; then
  printf '\n%d check(s) failed\n' "$failures"
  exit 1
fi
printf '\nAll checks passed\n'
