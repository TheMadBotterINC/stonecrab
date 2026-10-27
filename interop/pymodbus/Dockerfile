# Stonecrab interoperability peer (slice S27): a Modbus TCP server on pymodbus 3.8.6.
# First-party, Apache-2.0. Build with the repository root as the context:
#   docker build -f interop/pymodbus/Dockerfile -t stonecrab-interop:local .
# (interop/pymodbus/Dockerfile.dockerignore keeps the context to the three files below.)

# python:3.12-slim, the multi-architecture index digest pulled on 2026-10-27.
FROM python:3.12-slim@sha256:12316debbfb86890f6b2d825820612741ba527ad5a44e3902759141d572bfe8f

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_ROOT_USER_ACTION=ignore

WORKDIR /app

COPY interop/pymodbus/requirements.txt /app/requirements.txt
RUN pip install --no-cache-dir -r /app/requirements.txt

COPY interop/pymodbus/server.py /app/server.py
COPY fixtures/modbus/interop/register-map.toml /app/register-map.toml

# The server needs no privileges: port 5020 is above 1023 and it writes nothing.
RUN useradd --system --no-create-home --uid 10001 modbus
USER 10001

EXPOSE 5020
ENTRYPOINT ["python", "/app/server.py"]
