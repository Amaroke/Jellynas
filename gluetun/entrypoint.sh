#!/bin/sh
if [ -z "$VPN_SERVICE_PROVIDER" ]; then
  export VPN_SERVICE_PROVIDER=custom
  export VPN_TYPE=wireguard
  export WIREGUARD_ENDPOINT_IP=192.0.2.1
  export WIREGUARD_ENDPOINT_PORT=51820
  export WIREGUARD_PUBLIC_KEY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
  export WIREGUARD_PRIVATE_KEY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
  export WIREGUARD_ADDRESSES=10.64.0.2/32
fi
case "$GLUETUN_API_KEY" in
  *[!A-Za-z0-9]*)
    echo "GLUETUN_API_KEY holds a character other than a letter or a digit, the control server stays closed" >&2
    unset GLUETUN_API_KEY
    ;;
esac
if [ -n "$GLUETUN_API_KEY" ]; then
  export HTTP_CONTROL_SERVER_AUTH_CONFIG_FILEPATH=/tmp/control-server-auth.toml
  (
    umask 077
    printf '[[roles]]\nname = "homepage"\nroutes = ["GET /v1/publicip/ip"]\nauth = "apikey"\napikey = "%s"\n' "$GLUETUN_API_KEY" >"$HTTP_CONTROL_SERVER_AUTH_CONFIG_FILEPATH"
  )
  unset GLUETUN_API_KEY
fi
exec /gluetun-entrypoint "$@"
