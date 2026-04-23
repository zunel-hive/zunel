FROM ghcr.io/astral-sh/uv:python3.12-bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates git bubblewrap openssh-client && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Install dependencies first for better layer caching. The project metadata still
# expects legal-notice files during build, so create temporary placeholders here.
COPY pyproject.toml README.md ./
RUN mkdir -p zunel && touch zunel/__init__.py LICENSE THIRD_PARTY_NOTICES.md && \
    uv pip install --system --no-cache . && \
    rm -rf zunel LICENSE THIRD_PARTY_NOTICES.md

# Copy the full source and install the actual package.
COPY zunel/ zunel/
RUN touch LICENSE THIRD_PARTY_NOTICES.md && \
    uv pip install --system --no-cache . && \
    rm -f LICENSE THIRD_PARTY_NOTICES.md

# Create non-root user and config directory.
RUN useradd -m -u 1000 -s /bin/bash zunel && \
    mkdir -p /home/zunel/.zunel && \
    chown -R zunel:zunel /home/zunel /app

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN sed -i 's/\r$//' /usr/local/bin/entrypoint.sh && chmod +x /usr/local/bin/entrypoint.sh

USER zunel
ENV HOME=/home/zunel

EXPOSE 18790

ENTRYPOINT ["entrypoint.sh"]
CMD ["status"]
