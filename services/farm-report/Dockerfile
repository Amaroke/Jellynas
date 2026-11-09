FROM python:3.14.5-alpine3.22
RUN apk add --no-cache chromium font-dejavu tzdata
COPY report.py /app/report.py
CMD ["python", "-u", "/app/report.py"]
