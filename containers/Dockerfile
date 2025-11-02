#
# Bash shell with Rust nightly and address santizer support.
#
FROM rustlang/rust:nightly

RUN apt-get update && \
    apt-get -y install curl build-essential vim
RUN rustup component add rust-src --toolchain nightly-x86_64-unknown-linux-gnu

WORKDIR /leaker
COPY Cargo.toml Cargo.toml
COPY examples examples
COPY src src
RUN adduser --disabled-password --gecos '' leaker
RUN chown -R leaker .
USER leaker
ENV USER=leaker

COPY containers/leak.sh /leaker
