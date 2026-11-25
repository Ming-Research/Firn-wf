FROM debian:trixie-slim

# The Ubuntu 24.04 build needs a newer glibc than Debian bookworm provides.
# CI checks all dynamic dependencies in the finished runtime image.
RUN apt-get update \
    && apt-get install -y --no-install-recommends libgcc-s1 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data \
    && chown 10001:10001 /data

COPY firn /usr/local/bin/firn
COPY LICENSE /usr/local/share/licenses/firn/LICENSE
COPY commands/redis-license.txt /usr/local/share/licenses/firn/commands/redis-license.txt
COPY licenses/ /usr/local/share/licenses/firn/

USER 10001:10001
WORKDIR /data
VOLUME ["/data"]
EXPOSE 6379
ENTRYPOINT ["/usr/local/bin/firn"]
CMD ["--port", "6379", "--bind", "0.0.0.0", "--appendonly", "yes", "--appendfilename", "appendonly.aof"]
