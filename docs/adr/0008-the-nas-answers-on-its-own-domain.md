# The NAS answers on its own domain

Friends reached Jellyfin and Seerr through a random DuckDNS subdomain, and the Admin reached each tool over plain HTTP on its own port of the NAS. The NAS now answers on the domain of the Admin, hosted by OVH, over HTTPS only, through Caddy on a single port.

## Decision

- **Public names**: `jellyfin.<DOMAIN>` and `seerr.<DOMAIN>`, under a wildcard certificate for `*.<DOMAIN>`. Caddy keeps both records on the public IP of the home with its dynamic DNS app, in IPv4 only, so the DuckDNS service leaves.
- **Lan names**: every web UI, TrueNAS included, answers at `<tool>.lan.<DOMAIN>`, under a wildcard certificate for `*.lan.<DOMAIN>`. A static `*.lan` record points to the local IP of the NAS, and Caddy refuses a lan name to any address outside `LAN_SUBNET`.
- **One port**: Caddy is the only service that publishes a port, `CADDY_HTTPS_PORT`, on IPv4 only. The other services publish nothing and reach each other on the Compose network, as before.
- **Certificates through the OVH API**: Caddy proves it owns the names with a DNS challenge, using OVH keys that can only change the records of the zone.

## Considered Options

- **Keep DuckDNS for the public names, the domain for the lan names only**: rejected. It kept the public address apart from the identity of the Admin, but the certificate logs already list the DuckDNS name, the page behind it is a login, and the pseudonym of the Admin already names the public repo.
- **A certificate per name**: rejected. Each one lands in the certificate logs, which would list every tool of the NAS.
- **A port per tool**: rejected. Once every tool sits behind Caddy, a port per tool only widens what the NAS publishes.
- **A local certificate authority**: rejected. Each device, phones included, would need its root certificate, and a private suffix would need a local DNS server.
- **The private ranges for the lan names**: rejected. Docker relays the IPv6 and loopback connections through its own private addresses, which would pass for local ones, so the lan names only accept `LAN_SUBNET` and Caddy only listens on IPv4.

## Consequences

The addresses of Jellyfin and Seerr changed once, so every Friend updated their apps. The certificate logs show `jellyfin.<DOMAIN>`, `seerr.<DOMAIN>` and the two wildcards, never the name of a tool. A device on the local network reaches a lan name directly, while a public name loops through the router. The NAS depends on the OVH API to renew its certificates and to follow the public IP: with an expired key, the certificates still last up to 90 days, but a new public IP cuts Friends off until the key is renewed.
