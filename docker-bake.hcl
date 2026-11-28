group "default" {
  targets = ["caddy", "check", "dashboard", "discord-posts", "policy", "ratio", "ups"]
}

target "_image" {
  labels = {
    "org.opencontainers.image.source" = "https://github.com/Amaroke/Jellynas"
  }
}

target "caddy" {
  inherits = ["_image"]
  context  = "services/caddy"
  tags     = ["ghcr.io/amaroke/jellynas-caddy:main"]
}

target "check" {
  inherits = ["_image"]
  target   = "check"
  tags     = ["ghcr.io/amaroke/jellynas-check:main"]
}

target "dashboard" {
  inherits = ["_image"]
  target   = "dashboard"
  tags     = ["ghcr.io/amaroke/jellynas-dashboard:main"]
}

target "discord-posts" {
  inherits = ["_image"]
  target   = "discord-posts"
  tags     = ["ghcr.io/amaroke/jellynas-discord-posts:main"]
}

target "policy" {
  inherits = ["_image"]
  target   = "policy"
  tags     = ["ghcr.io/amaroke/jellynas-policy:main"]
}

target "ratio" {
  inherits = ["_image"]
  target   = "ratio"
  tags     = ["ghcr.io/amaroke/jellynas-ratio:main"]
}

target "ups" {
  inherits = ["_image"]
  target   = "ups"
  tags     = ["ghcr.io/amaroke/jellynas-ups:main"]
}
