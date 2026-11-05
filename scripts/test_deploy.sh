#!/usr/bin/env bash
set -uo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
failures=0

setup() {
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/bin"
  cat > "$tmp/bin/docker" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$STUB_DIR/docker.log"
case "$*" in
  "compose config --services") cat "$STUB_DIR/services" ;;
  "compose up"*) [ ! -f "$STUB_DIR/fail-up" ] || exit 1 ;;
esac
exit 0
EOF
  chmod +x "$tmp/bin/docker"
  printf 'jellyfin\nratio-limit\nratio-grab\n' > "$tmp/services"
  export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.autocrlf GIT_CONFIG_VALUE_0=false
  git init -q --bare -b main "$tmp/origin.git"
  git clone -q "$tmp/origin.git" "$tmp/seed" 2>/dev/null
  mkdir -p "$tmp/seed/scripts" "$tmp/seed/jellyfin" "$tmp/seed/ratio-limit" "$tmp/seed/tdarr"
  cp "$repo/scripts/deploy.sh" "$tmp/seed/scripts/deploy.sh"
  printf 'services:\n  jellyfin:\n    image: jellyfin\n' > "$tmp/seed/jellyfin/compose.yaml"
  printf 'services:\n  ratio-limit:\n    image: python\n\n  ratio-grab:\n    image: python\nnetworks:\n  other:\n' > "$tmp/seed/ratio-limit/compose.yaml"
  printf 'print(1)\n' > "$tmp/seed/ratio-limit/ratio_grab.py"
  printf 'services:\n  tdarr:\n    image: tdarr\n' > "$tmp/seed/tdarr/compose.yaml"
  printf '{}\n' > "$tmp/seed/tdarr/flow.json"
  printf '# NAS\n' > "$tmp/seed/README.md"
  commit "initial"
  git clone -q "$tmp/origin.git" "$tmp/nas"
}

teardown() {
  rm -rf "$tmp"
}

git_seed() {
  git -C "$tmp/seed" -c user.name=test -c user.email=test@example.com "$@"
}

commit() {
  git_seed add -A
  git_seed commit -q -m "$1"
  git_seed push -q origin main 2>/dev/null
}

deploy() {
  : > "$tmp/docker.log"
  STUB_DIR="$tmp" PATH="$tmp/bin:$PATH" "$tmp/nas/scripts/deploy.sh" > "$tmp/out.log" 2>&1
}

docker_calls() {
  cat "$tmp/docker.log"
}

check() {
  if [ "$2" = "$3" ]; then
    printf 'PASS %s\n' "$1"
  else
    printf 'FAIL %s\n  expected: %s\n  actual:   %s\n' "$1" "$(printf '%s' "$3" | tr '\n' '|')" "$(printf '%s' "$2" | tr '\n' '|')"
    sed 's/^/  | /' "$tmp/out.log"
    failures=$((failures + 1))
  fi
}

up="compose up -d --build --remove-orphans"

test_first_run_starts_everything() {
  setup
  deploy
  check "first run exits 0" "$?" 0
  check "first run starts every service" "$(docker_calls)" "$up"
  teardown
}

test_nothing_new_does_nothing() {
  setup
  deploy
  deploy
  check "run without a new commit exits 0" "$?" 0
  check "run without a new commit calls no docker" "$(docker_calls)" ""
  teardown
}

test_first_run_behind_restarts_pulled_files() {
  setup
  printf 'print(2)\n' > "$tmp/seed/ratio-limit/ratio_grab.py"
  commit "grab"
  deploy
  check "first run behind restarts the services of the files it pulled" "$(docker_calls)" "$up
compose config --services
compose restart ratio-limit ratio-grab"
  teardown
}

test_new_commit_is_pulled() {
  setup
  deploy
  printf '# NAS v2\n' > "$tmp/seed/README.md"
  commit "readme"
  deploy
  check "new commit is fast-forwarded" "$(git -C "$tmp/nas" rev-parse HEAD)" "$(git_seed rev-parse HEAD)"
  check "doc-only commit only runs compose up" "$(docker_calls)" "$up"
  teardown
}

test_mounted_file_restarts_its_services() {
  setup
  deploy
  printf 'print(2)\n' > "$tmp/seed/ratio-limit/ratio_grab.py"
  commit "grab"
  deploy
  check "changed mounted file restarts the services of its folder" "$(docker_calls)" "$up
compose config --services
compose restart ratio-limit ratio-grab"
  teardown
}

test_compose_change_does_not_restart() {
  setup
  deploy
  printf 'services:\n  jellyfin:\n    image: jellyfin:2\n' > "$tmp/seed/jellyfin/compose.yaml"
  commit "bump"
  deploy
  check "compose-only change relies on compose up" "$(docker_calls)" "$up"
  teardown
}

test_disabled_service_is_not_restarted() {
  setup
  deploy
  printf '{"v":2}\n' > "$tmp/seed/tdarr/flow.json"
  commit "flow"
  deploy
  check "service missing from the config is not restarted" "$(docker_calls)" "$up
compose config --services"
  teardown
}

test_local_change_blocks_deploy() {
  setup
  deploy
  printf '# local\n' > "$tmp/nas/README.md"
  printf '# NAS v2\n' > "$tmp/seed/ratio-limit/ratio_grab.py"
  commit "upstream"
  local before
  before="$(git -C "$tmp/nas" rev-parse HEAD)"
  deploy
  check "local change makes the run fail" "$?" 1
  check "local change keeps HEAD" "$(git -C "$tmp/nas" rev-parse HEAD)" "$before"
  check "local change calls no docker" "$(docker_calls)" ""
  teardown
}

test_wrong_branch_blocks_deploy() {
  setup
  git -C "$tmp/nas" switch -q -c other
  deploy
  check "other branch makes the run fail" "$?" 1
  check "other branch calls no docker" "$(docker_calls)" ""
  teardown
}

test_failed_up_is_retried() {
  setup
  deploy
  printf 'print(3)\n' > "$tmp/seed/ratio-limit/ratio_grab.py"
  commit "grab"
  touch "$tmp/fail-up"
  deploy
  check "failed compose up makes the run fail" "$?" 1
  check "failed compose up skips the restarts" "$(docker_calls)" "$up"
  rm "$tmp/fail-up"
  deploy
  check "next run retries the failed deploy" "$(docker_calls)" "$up
compose config --services
compose restart ratio-limit ratio-grab"
  teardown
}

test_first_run_starts_everything
test_nothing_new_does_nothing
test_first_run_behind_restarts_pulled_files
test_new_commit_is_pulled
test_mounted_file_restarts_its_services
test_compose_change_does_not_restart
test_disabled_service_is_not_restarted
test_local_change_blocks_deploy
test_wrong_branch_blocks_deploy
test_failed_up_is_retried

if [ "$failures" -gt 0 ]; then
  printf '\n%d check(s) failed\n' "$failures"
  exit 1
fi
printf '\nAll checks passed\n'
