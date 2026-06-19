# Application image for the gb-grid ingester / CLI.
#
# Only *our* code goes in here. Postgres (TimescaleDB) and Grafana are pulled as
# upstream images by docker-compose at run time, so nothing in this repo
# redistributes them.
FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

WORKDIR /app

# All runtime deps ship manylinux wheels (psycopg[binary], psycopg2-binary,
# pandas, pydantic-core), so no compiler/system libpq is needed in the image.
COPY pyproject.toml README.md ./
COPY src ./src

RUN pip install .

# Defaults; docker-compose overrides the command for the ingester vs. one-off
# CLI invocations (backfill, status, sql, ...).
ENTRYPOINT ["gb-grid"]
CMD ["--help"]
