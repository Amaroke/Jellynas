#!/usr/bin/env bash
set -uo pipefail

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
  value="$(grep -E "^$1=" .env 2>/dev/null | tail -n 1 | cut -d= -f2- | sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'$/\1/")"
  while [[ "$value" =~ \$\{([A-Z_]+)\} ]]; do
    ref="${BASH_REMATCH[1]}"
    [ "$ref" != "$1" ] || break
    value="${value//\$\{$ref\}/$(env_value "$ref")}"
  done
  printf '%s' "$value"
}

fetch_in() {
  local service="$1" url="$2" seconds="$3"
  docker compose exec -T "$service" sh -c 'if command -v curl >/dev/null 2>&1; then curl --silent --max-time "$2" "$1"; elif command -v wget >/dev/null 2>&1; then wget -q -t 1 -T "$2" -O - "$1" || exit 7; else exit 127; fi' sh "$url" "$seconds" 2>/dev/null
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

enabled() {
  grep -qx "$1" <<<"$services"
}

included_compose_files() {
  sed -n 's/^  - \([^ #]*compose\.yaml\)$/\1/p' compose.yaml
}

check_env_filled() {
  local name value files missing=""
  if [ ! -f .env ]; then
    fail env ".env is missing, copy .env.example and fill it in"
    return
  fi
  mapfile -t files < <(included_compose_files)
  while read -r name; do
    value="$(env_value "$name")"
    [ -n "$value" ] || missing="$missing $name"
  done < <(grep -ohE '[$][{][A-Z_]+[}]' "${files[@]}" | tr -d '$\{}' | sort -u)
  if [ -n "$missing" ]; then
    fail env "empty or missing in .env:$missing"
  else
    pass env "every variable the enabled services require is set in .env"
  fi
}

check_containers_up() {
  local service state deadline
  if [ -z "$services" ]; then
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
    case "$service:$state" in
      *:running/healthy | *:running)
        pass "$service" "container is $state"
        ;;
      gluetun:running/unhealthy)
        if [ -z "$vpn_provider" ]; then
          pass gluetun "container is $state, expected while the VPN is not configured"
        else
          fail gluetun "container is $state, the VPN does not connect"
        fi
        ;;
      *)
        fail "$service" "container is $state, expected healthy or running"
        ;;
    esac
  done
}

check_doco_cd() {
  local state
  state="$(docker inspect -f '{{.State.Status}}' doco-cd 2>/dev/null)"
  if [ "$state" = running ]; then
    pass doco-cd "container is running, the NAS follows main"
  else
    fail doco-cd "container is ${state:-missing}, the NAS no longer follows main"
  fi
}

check_read_only() {
  local service="$1" path="$2" probe
  enabled "$service" || return
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

data_mounts() {
  local service="$1" promise="$2" data_root id source
  enabled "$service" || return 1
  data_root="$(env_value DATA_ROOT)"
  data_root="${data_root%/}"
  if [ -z "$data_root" ]; then
    fail "$service" "DATA_ROOT is empty, cannot check that it $promise"
    return 1
  fi
  case "$(container_state "$service")" in
    running*) ;;
    *)
      fail "$service" "is not running, cannot check that it $promise"
      return 1
      ;;
  esac
  id="$(docker compose ps -q "$service" 2>/dev/null | head -n 1)"
  while IFS= read -r source; do
    case "$data_root" in
      "$source"/*) printf '/ %s\n' "$source" ;;
    esac
    case "$source" in
      "$data_root") printf '/ %s\n' "$source" ;;
      "$data_root"/*) printf '%s %s\n' "${source#"$data_root"/}" "$source" ;;
    esac
  done < <(docker inspect -f '{{range .Mounts}}{{println .Source}}{{end}}' "$id")
}

check_not_mounted() {
  local service="$1" subpath="$2" promise="$3" mounts relative source mounted=""
  mounts="$(data_mounts "$service" "$promise")" || return
  while read -r relative source; do
    [ -n "$source" ] || continue
    if [ -z "$subpath" ]; then
      mounted="$mounted $source"
    else
      case "$relative" in
        / | "$subpath" | "$subpath"/*) mounted="$mounted $source" ;;
      esac
    fi
  done <<<"$mounts"
  if [ -n "$mounted" ]; then
    fail "$service" "mounts$mounted, expected that it $promise"
  else
    pass "$service" "$promise"
  fi
}

check_mounts_only() {
  local service="$1" mounts relative source allowed path stray=""
  shift
  mounts="$(data_mounts "$service" "mounts only $*")" || return
  while read -r relative source; do
    [ -n "$source" ] || continue
    allowed=""
    for path in "$@"; do
      case "$relative" in
        "$path" | "$path"/*) allowed=yes ;;
      esac
    done
    [ -n "$allowed" ] || stray="$stray $source"
  done <<<"$mounts"
  if [ -n "$stray" ]; then
    fail "$service" "mounts$stray, expected only $*"
  else
    pass "$service" "mounts only $*"
  fi
}

check_no_media_mount() {
  check_not_mounted "$1" "" "has no media mount"
}

check_no_media_tree() {
  check_not_mounted "$1" media "does not mount the media tree"
}

check_no_torrents_mount() {
  check_not_mounted "$1" torrents "does not mount the torrents tree"
}

check_published_ports() {
  local service id ports bindings exposed=""
  for service in $services; do
    id="$(docker compose ps -q "$service" 2>/dev/null | head -n 1)"
    [ -n "$id" ] || continue
    ports="$(docker inspect -f '{{range $p, $b := .HostConfig.PortBindings}}{{range $b}} {{.HostPort}}{{end}}{{end}}' "$id")"
    if [ "$service" = caddy ]; then
      bindings="$(docker inspect -f '{{range $p, $b := .HostConfig.PortBindings}}{{range $b}} {{.HostIp}}:{{.HostPort}}{{end}}{{end}}' "$id")"
      [ "$bindings" = " 0.0.0.0:$https_port" ] || exposed="$exposed caddy($bindings )"
    elif [ -n "$ports" ]; then
      exposed="$exposed $service($ports )"
    fi
  done
  if [ -n "$exposed" ]; then
    fail compose "only caddy may publish a port, $https_port on IPv4, found:$exposed"
  else
    pass compose "only caddy publishes a port, $https_port on IPv4"
  fi
}

check_no_new_privileges() {
  local service id missing=""
  for service in $services; do
    id="$(docker compose ps -q "$service" 2>/dev/null | head -n 1)"
    [ -n "$id" ] || continue
    docker inspect -f '{{.HostConfig.SecurityOpt}}' "$id" | grep -q no-new-privileges || missing="$missing $service"
  done
  if [ -n "$missing" ]; then
    fail compose "may gain privileges, no-new-privileges is missing on:$missing"
  else
    pass compose "every service runs with no-new-privileges"
  fi
}

check_docker_socket_read_only() {
  local service id writable=""
  for service in $services; do
    id="$(docker compose ps -q "$service" 2>/dev/null | head -n 1)"
    [ -n "$id" ] || continue
    docker inspect -f '{{range .Mounts}}{{if .RW}}{{println .Source}}{{end}}{{end}}' "$id" | grep -q 'docker\.sock$' && writable="$writable $service"
  done
  if [ -n "$writable" ]; then
    fail compose "mounts the Docker socket read-write:$writable"
  else
    pass compose "no service mounts the Docker socket read-write"
  fi
}

check_homepage_values() {
  local name unsafe=""
  enabled homepage || return
  while read -r name; do
    case "$(env_value "$name")" in
      *"'"*) unsafe="$unsafe $name" ;;
    esac
  done < <(sed -n 's/^      HOMEPAGE_VAR_[A-Z_]*: [$][{]\([A-Z_]*\).*/\1/p' homepage/compose.yaml)
  if [ -n "$unsafe" ]; then
    fail homepage "a single quote in$unsafe breaks its config, the page stays empty"
  else
    pass homepage "no value it reads from .env holds a single quote"
  fi
}

check_login_required() {
  local service="$1" url="$2" host result
  enabled "$service" || return
  case "$(container_state "$service")" in
    running*) ;;
    *)
      fail "$service" "is not running, cannot check that its web UI requires a login"
      return
      ;;
  esac
  host="${url#https://}"
  host="${host%%[:/]*}"
  result="$(curl --silent --output /dev/null --write-out '%{http_code} %{redirect_url}' --max-time 10 --resolve "$host:$https_port:$host_ip" "$url")"
  case "$result" in
    30?" http://"*)
      fail "$service" "web UI redirects to plain HTTP at $url (${result#* })"
      ;;
    401" "* | 403" "*)
      pass "$service" "web UI requires a login (HTTP ${result%% *})"
      ;;
    30?" "*/login* | 30?" "*/signin*)
      pass "$service" "web UI requires a login (redirects to its login page)"
      ;;
    *)
      fail "$service" "web UI does not require a login at $url (HTTP ${result%% *})"
      ;;
  esac
}

check_api_key_required() {
  local service="$1" url="$2" status
  enabled "$service" || return
  case "$(container_state "$service")" in
    running*) ;;
    *)
      fail "$service" "is not running, cannot check that its API requires a key"
      return
      ;;
  esac
  status="$(docker compose exec -T caddy wget --server-response --spider "$url" 2>&1 | grep -oE 'HTTP/[0-9.]+ [0-9]{3}' | tail -n 1 | cut -d' ' -f2)"
  case "$status" in
    401 | 403) pass "$service" "API requires a key (HTTP $status)" ;;
    *) fail "$service" "API answers ${status:-nothing} without a key at $url" ;;
  esac
}

check_exit_ip() {
  local service="$1" home vpn
  [ -n "$vpn_provider" ] && enabled "$service" || return
  case "$(container_state "$service")" in
    running*) ;;
    *)
      fail "$service" "is not running, cannot check that it goes through the VPN"
      return
      ;;
  esac
  home="$(curl --silent --max-time 10 https://api.ipify.org)"
  vpn="$(fetch_in "$service" https://api.ipify.org 10)"
  if [ -z "$vpn" ]; then
    fail "$service" "cannot reach the internet through the VPN"
  elif [ -z "$home" ]; then
    fail "$service" "cannot get the home IP to compare with the VPN IP"
  elif [ "$vpn" = "$home" ]; then
    fail "$service" "goes out with the home IP, not through the VPN"
  else
    pass "$service" "goes out through the VPN, not with the home IP"
  fi
}

check_kill_switch() {
  local service probed=() pids=() i state
  enabled gluetun || return
  for service in "$@"; do
    enabled "$service" || continue
    case "$(container_state gluetun) $(container_state "$service")" in
      running*" "running*) probed+=("$service") ;;
      *) fail "$service" "gluetun or $service is not running, cannot check the kill switch" ;;
    esac
  done
  [ "${#probed[@]}" -gt 0 ] || return
  if ! docker compose exec -T gluetun ip link set tun0 down >/dev/null 2>&1; then
    fail gluetun "cannot take the VPN interface tun0 down, cannot check the kill switch"
    return
  fi
  for service in "${probed[@]}"; do
    fetch_in "$service" http://1.1.1.1 5 </dev/null >/dev/null &
    pids+=("$!")
  done
  for i in "${!probed[@]}"; do
    wait "${pids[$i]}"
    state="$?"
    service="${probed[$i]}"
    case "$state" in
      0) fail "$service" "reaches the internet with the VPN down" ;;
      6 | 7 | 28) pass "$service" "has no connectivity with the VPN down" ;;
      *) fail "$service" "cannot probe its connectivity with curl or wget" ;;
    esac
  done
  if docker compose exec -T gluetun ip -br link show tun0 2>/dev/null </dev/null | grep -qv DOWN; then
    fail gluetun "brought the VPN back up during the probes, the kill switch results above are not reliable"
  fi
  docker compose restart gluetun "${probed[@]}" >/dev/null 2>&1
}

check_slskd_port() {
  local port forwarded
  enabled slskd || return
  port="$(env_value SLSKD_LISTEN_PORT)"
  port="${port:-50300}"
  forwarded="$(env_value VPN_FORWARDED_PORT)"
  forwarded="${forwarded// /}"
  if [[ "$forwarded" == *,* && ",${forwarded#*,}," == *",$port,"* ]]; then
    pass slskd "listens on $port, a port Gluetun opens from VPN_FORWARDED_PORT"
  else
    fail slskd "listens on $port, not one of VPN_FORWARDED_PORT after the qBittorrent one, other Soulseek users cannot reach it"
  fi
}

check_backup_root() {
  local backup_root name root
  enabled backup || return
  backup_root="$(env_value BACKUP_ROOT)"
  if [ ! -d "$backup_root" ]; then
    fail backup "BACKUP_ROOT is not a folder of the Host, the archives have nowhere to go"
    return
  fi
  for name in DATA_ROOT APPDATA_ROOT; do
    root="$(env_value "$name")"
    root="${root%/}"
    if [ ! -d "$root" ]; then
      fail backup "$name is not a folder of the Host, cannot check that BACKUP_ROOT is on another dataset"
    elif [ "${backup_root%/}" = "$root" ] || [[ "$backup_root" == "$root"/* ]]; then
      fail backup "BACKUP_ROOT is inside $name, the archives would land in what they back up"
    elif [ "$(stat -c %d "$backup_root")" = "$(stat -c %d "$root")" ]; then
      fail backup "BACKUP_ROOT is on the same dataset as $name, one loss takes both"
    else
      pass backup "BACKUP_ROOT is on another dataset than $name"
    fi
  done
}

caddy_running() {
  enabled caddy || return 1
  case "$(container_state caddy)" in
    running*) return 0 ;;
  esac
  fail caddy "is not running, cannot check $1"
  return 1
}

check_public_hostname() {
  local host="$1" target="${2:-127.0.0.1}" code deadline redirect
  [ -n "$host" ] || return
  caddy_running "https://$host" || return
  deadline=$((SECONDS + health_timeout))
  code="$(http_code --resolve "$host:$https_port:$target" "https://$host:$https_port/")"
  while { [ "$code" = 000 ] || [ "${code#5}" != "$code" ]; } && [ "$SECONDS" -lt "$deadline" ]; do
    sleep 5
    code="$(http_code --resolve "$host:$https_port:$target" "https://$host:$https_port/")"
  done
  redirect="$(curl --silent --output /dev/null --write-out '%{redirect_url}' --max-time 10 --resolve "$host:$https_port:$target" "https://$host:$https_port/")"
  case "$code:$redirect" in
    3??:http://*)
      fail caddy "https://$host redirects to plain HTTP ($redirect)"
      ;;
    2??:* | 3??:*)
      pass caddy "https://$host answers $code with a valid certificate"
      ;;
    *)
      fail caddy "https://$host does not answer with a valid certificate (HTTP $code)"
      ;;
  esac
}

check_lan_refused_outside() {
  local host="$1" code
  caddy_running "that $host is refused outside the local network" || return
  code="$(http_code --insecure --resolve "$host:$https_port:127.0.0.1" "https://$host:$https_port/")"
  if [ "$code" = 000 ]; then
    pass caddy "refuses https://$host from outside $lan_subnet"
  else
    fail caddy "answers $code to https://$host from outside $lan_subnet"
  fi
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

services="$(docker compose config --services 2>/dev/null)"
domain="$(env_value DOMAIN)"
lan_subnet="$(env_value LAN_SUBNET)"
lan="lan.$domain"
jellyfin_hostname="${domain:+jellyfin.$domain}"
seerr_hostname="${domain:+seerr.$domain}"
vpn_provider="$(env_value VPN_SERVICE_PROVIDER)"
host_ip="$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p')"
https_port="$(env_value CADDY_HTTPS_PORT)"
https_port="${https_port:-443}"
public_port=""
[ "$https_port" = 443 ] || public_port=":$https_port"

check_compose_renders
check_env_filled
check_containers_up
check_read_only jellyfin /data/media
check_doco_cd
check_no_media_mount caddy
check_no_media_mount gluetun
check_no_torrents_mount jellyfin
check_no_media_tree qbittorrent
check_mounts_only lidarr media/music torrents
check_mounts_only slskd media/music torrents/soulseek
check_read_only slskd /data/media/music
check_mounts_only music-bot media/music
check_read_only music-bot /data/media/music
check_slskd_port
check_mounts_only lazylibrarian media/books torrents
check_mounts_only suwayomi media/manga
check_no_media_mount seerr
check_no_media_mount prowlarr
check_no_media_mount flaresolverr
check_no_media_mount recyclarr
check_no_torrents_mount bazarr
check_no_torrents_mount tdarr
check_mounts_only indexarr media/movies media/shows media/anime
check_read_only indexarr /data/media/movies
check_read_only indexarr /data/media/shows
check_read_only indexarr /data/media/anime
check_no_media_mount autobrr
check_no_media_tree cross-seed
check_no_media_mount ratio
check_no_media_mount beszel
check_no_media_tree beszel-agent
check_no_torrents_mount beszel-agent
check_no_media_mount uptime-kuma
check_no_media_mount autokuma
check_no_media_mount dozzle
check_no_media_tree homepage
check_no_torrents_mount homepage
check_no_media_mount backup
check_read_only backup /backup/appdata
check_backup_root
check_homepage_values
check_published_ports
check_no_new_privileges
check_docker_socket_read_only
check_login_required qbittorrent "https://qbittorrent.$lan:$https_port/api/v2/app/version"
check_login_required prowlarr "https://prowlarr.$lan:$https_port/"
check_login_required radarr "https://radarr.$lan:$https_port/"
check_login_required sonarr "https://sonarr.$lan:$https_port/"
check_login_required lidarr "https://lidarr.$lan:$https_port/"
check_login_required lazylibrarian "https://lazylibrarian.$lan:$https_port/home"
check_login_required suwayomi "https://suwayomi.$lan:$https_port/api/graphql"
check_login_required slskd "https://slskd.$lan:$https_port/api/v0/application"
check_login_required bazarr "https://bazarr.$lan:$https_port/bazarr.log"
check_login_required tdarr "https://tdarr.$lan:$https_port/api/v2/get-nodes"
check_login_required indexarr "https://indexarr.$lan:$https_port/api/stats"
check_login_required autobrr "https://autobrr.$lan:$https_port/api/config"
check_login_required beszel "https://beszel.$lan:$https_port/api/beszel/info"
check_login_required uptime-kuma "https://uptime-kuma.$lan:$https_port/metrics"
check_login_required dozzle "https://dozzle.$lan:$https_port/"
check_login_required homepage "https://homepage.$lan:$https_port/api/services"
check_api_key_required cross-seed "http://cross-seed:2468/api/status"
check_exit_ip qbittorrent
check_exit_ip slskd
check_exit_ip suwayomi
check_kill_switch qbittorrent slskd suwayomi
check_public_hostname "$jellyfin_hostname"
check_public_hostname "$seerr_hostname"
check_public_hostname "${domain:+truenas.$lan}" "$host_ip"
check_public_hostname "${domain:+jellyfin.$lan}" "$host_ip"
check_public_hostname "${domain:+seerr.$lan}" "$host_ip"
check_public_hostname "${domain:+qbittorrent.$lan}" "$host_ip"
check_public_hostname "${domain:+homepage.$lan}" "$host_ip"
check_lan_refused_outside "radarr.$lan"
check_lan_refused_outside "truenas.$lan"
check_lan_refused_outside "homepage.$lan"
check_unknown_host_refused "$jellyfin_hostname"

if [ -n "$jellyfin_hostname$seerr_hostname" ]; then
  printf '\nOn a phone on 4G with wifi off, open each address and check that it loads:\n'
  for host in "$jellyfin_hostname" "$seerr_hostname"; do
    [ -z "$host" ] || printf '  https://%s%s\n' "$host" "$public_port"
  done
fi

if [ "$failures" -gt 0 ]; then
  printf '\n%d check(s) failed\n' "$failures"
  exit 1
fi
printf '\nAll checks passed\n'
