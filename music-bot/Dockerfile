FROM python:3.14.5-slim-trixie
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ffmpeg libopus0 \
    && rm -rf /var/lib/apt/lists/*
COPY requirements.txt /app/requirements.txt
RUN pip install --no-cache-dir --requirement /app/requirements.txt
COPY bot.py jellyfin.py player.py rules.py status.py /app/
CMD ["python", "-u", "/app/bot.py"]
