FROM python:3.15.2-slim-trixie
RUN pip install --no-cache-dir pillow==13.0.0
COPY covers.py /app/covers.py
CMD ["python", "-u", "/app/covers.py"]
