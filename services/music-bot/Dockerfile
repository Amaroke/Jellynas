FROM python:3.15.2-slim-trixie
# renovate: datasource=deb depName=ffmpeg versioning=deb registryUrl=https://deb.debian.org/debian?suite=trixie&components=main&binaryArch=amd64
ARG FFMPEG_VERSION=7:7.1.5-0+deb13u1
# renovate: datasource=deb depName=libopus0 versioning=deb registryUrl=https://deb.debian.org/debian?suite=trixie&components=main&binaryArch=amd64
ARG LIBOPUS0_VERSION=1.5.2-2
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
    ffmpeg="${FFMPEG_VERSION}" \
    libopus0="${LIBOPUS0_VERSION}" \
    && rm -rf /var/lib/apt/lists/*
COPY requirements.txt /app/requirements.txt
RUN pip install --no-cache-dir --requirement /app/requirements.txt
COPY bot.py jellyfin.py player.py rules.py status.py /app/
CMD ["python", "-u", "/app/bot.py"]
