# The NAS is split into network zones

Every service shared one Compose network, so a flaw in Jellyfin or Seerr, the two services that face the internet, landed next to Radarr, Sonarr, qBittorrent, Dozzle, Homepage and the rest. The lan names of Caddy only guarded the way in from outside, not the traffic between containers. The NAS now sits in three Network zones.

## Decision

- **Three zones**: `jellyfin-zone` holds Caddy and Jellyfin. `seerr-zone` holds Caddy, Seerr, Radarr and Sonarr, the services Seerr sends Requests to. `internal-zone` holds every other service, Caddy, Radarr and Sonarr included. None is a Docker internal network, since Jellyfin and Seerr fetch metadata from the internet. The default Compose network is no longer used.
- **Access through the Public names**: a service that needs Jellyfin or Seerr reaches it through Caddy at its Public name, as a Friend does. On `internal-zone` and `seerr-zone`, Caddy carries the network aliases `jellyfin.<DOMAIN>` and `seerr.<DOMAIN>`, so a container resolves them straight to Caddy, without a loop through the router. The zones only open towards Jellyfin and Seerr, never from them.
- **One port inside and outside**: Caddy listens on `CADDY_HTTPS_PORT` inside its container too, so `https://jellyfin.<DOMAIN>:<CADDY_HTTPS_PORT>` is one address from everywhere.
- **A read-only Docker socket proxy**: Dozzle, Uptime Kuma, AutoKuma and the Beszel agent only watch Docker. They reach it through a proxy that lets through reads only, in `internal-zone`, and on `127.0.0.1` for the Beszel agent, which runs in the network of the Host. Only the proxy, backup and doco-cd mount the socket.

## Considered Options

- **One shared network**: rejected. It is what the NAS had, and a flaw in Jellyfin reached every service directly.
- **One network per function**, such as automation, VPN or observation: rejected. Homepage, Uptime Kuma and Caddy talk to nearly every service, so they would join every network and the zones would protect little, for many more networks to keep right.
- **Seerr in the internal zone**: rejected. Seerr faces the internet as Jellyfin does, and only needs Radarr, Sonarr and Jellyfin.
- **Jellyfin and Seerr by their container names, Caddy joining every zone as a router**: rejected. A service in the internal zone would then need to join the zone of Jellyfin, which opens the way back.

## Consequences

Homepage, Uptime Kuma, the Music bot and Seerr call Jellyfin at its Public name over HTTPS, which looks odd for two containers on one Host but is on purpose. A monitor of Jellyfin or Seerr checks Caddy, its certificate and the service, but no longer the port forward of the router nor the DNS record. Deploying the split recreates almost every service at once. Jellyfin looks up the address of Caddy for its known proxies when it starts, so it restarts after Caddy is recreated. `python3 -m unittest discover -s tests` fails when a service joins the wrong zone, names Jellyfin or Seerr directly, mounts the Docker socket or publishes a port beyond the loopback.
