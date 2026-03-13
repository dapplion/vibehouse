# `lcli` requires the full project to be in scope, so this should be built either:
#  - from the vibehouse dir with the command: `docker build -f ./lcli/Dockerfile .`
#  - from the current directory with the command: `docker build -f ./Dockerfile ../`
FROM rust:1.91.0-trixie AS builder
RUN apt-get update && apt-get -y upgrade && apt-get install -y cmake libclang-dev
COPY . vibehouse
ARG FEATURES
ENV FEATURES=$FEATURES
RUN cd vibehouse && make install-lcli

FROM ubuntu:22.04
RUN apt-get update && apt-get -y upgrade && apt-get clean && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/lcli /usr/local/bin/lcli
