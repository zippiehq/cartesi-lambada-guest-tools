FROM ubuntu:latest

RUN apt-get update && \
    apt-get install -y curl && \
    apt-get install -y curl build-essential wget && \
    apt-get clean

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

ENV PATH="/root/.cargo/bin:${PATH}"

RUN wget https://dist.ipfs.io/go-ipfs/v0.9.0/go-ipfs_v0.9.0_linux-amd64.tar.gz && \
    tar -xvzf go-ipfs_v0.9.0_linux-amd64.tar.gz && \
    bash go-ipfs/install.sh && \
    ipfs init --profile=server

COPY ./rollup-http-server /app/rollup-http-server
COPY ./rollup-http-client /app/rollup-http-client

WORKDIR /app/rollup-http-server

ENV USE_ROLLUP_BINDINGS_MOCK=1
ENV CACHE_DIR=output/
ENV STORE_DIR=/output
ENV IO_DEVICE=/dev/mtdblock1
RUN chmod +x build.sh

CMD sh -c "ipfs daemon & sleep 10 && cargo test -- --show-output --test-threads=1"