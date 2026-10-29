# Stonecrab interoperability peer (slice S52c): a multi-unit Modbus TCP server on modbus-tk
# 1.1.5. The server script is first-party, Apache-2.0; modbus-tk is LGPL-2.1-or-later and is
# installed from PyPI, not copied. Build with the repository root as the context:
#   docker build -f interop/modbus-tk/Dockerfile -t stonecrab-interop-modbus-tk:local .
# (interop/modbus-tk/Dockerfile.dockerignore keeps the context to the two files below.)

# python:3.12-slim, the multi-architecture index digest pulled on 2026-10-27 (the base of
# interop/pymodbus/Dockerfile).
FROM python:3.12-slim@sha256:12316debbfb86890f6b2d825820612741ba527ad5a44e3902759141d572bfe8f

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_ROOT_USER_ACTION=ignore

WORKDIR /app

COPY interop/modbus-tk/requirements.txt /app/requirements.txt
RUN pip install --no-cache-dir -r /app/requirements.txt

COPY interop/modbus-tk/server.py /app/server.py

# The server needs no privileges: port 5030 is above 1023 and it writes nothing.
RUN useradd --system --no-create-home --uid 10001 modbus
USER 10001

EXPOSE 5030
ENTRYPOINT ["python", "/app/server.py"]
