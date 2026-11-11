FROM python:3.14.5-alpine3.22
# renovate: datasource=repology depName=alpine_3_22/chromium versioning=loose
ARG CHROMIUM_VERSION=142.0.7444.59-r0
# renovate: datasource=repology depName=alpine_3_22/font-dejavu versioning=loose
ARG FONT_DEJAVU_VERSION=2.37-r6
# renovate: datasource=repology depName=alpine_3_22/tzdata versioning=loose
ARG TZDATA_VERSION=2026f-r0
RUN apk add --no-cache \
    chromium="${CHROMIUM_VERSION}" \
    font-dejavu="${FONT_DEJAVU_VERSION}" \
    tzdata="${TZDATA_VERSION}"
COPY board.py page.py /app/
CMD ["python", "-u", "/app/board.py"]
