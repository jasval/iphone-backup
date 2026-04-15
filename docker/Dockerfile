# Build & push to Docker Hub (run from project root):
#   docker buildx build --platform linux/amd64 -t youruser/iphone-backup:latest -f docker/Dockerfile --push .
#
# If building directly on the NAS via SSH (no cross-compilation needed):
#   docker build -t youruser/iphone-backup:latest -f docker/Dockerfile .

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libimobiledevice-utils \
    usbmuxd \
    python3 \
    python3-pip \
    python3-venv \
    python3-dev \
    cron \
    curl \
    jq \
    rsync \
    gcc \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN python3 -m venv /opt/pymobile && \
    /opt/pymobile/bin/pip install --upgrade pip pymobiledevice3

ENV PATH="/opt/pymobile/bin:$PATH"

COPY scripts/ibackup.sh     /usr/local/bin/ibackup.sh
COPY scripts/setup.sh       /usr/local/bin/setup.sh
COPY scripts/entrypoint.sh  /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/ibackup.sh \
             /usr/local/bin/setup.sh \
             /usr/local/bin/entrypoint.sh

VOLUME ["/backups"]
CMD ["/usr/local/bin/entrypoint.sh"]
