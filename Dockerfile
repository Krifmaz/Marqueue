# Builds for the platform Docker targets (amd64 or arm64 via buildx).
FROM rust:1.97-alpine AS build
RUN apk add --no-cache musl-dev git
WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin marqueue

FROM alpine:3.22
RUN apk add --no-cache ca-certificates ffmpeg
COPY --from=build /src/target/release/marqueue /usr/local/bin/marqueue
ENV MARQUEUE_LISTEN=0.0.0.0:7979
VOLUME /config
EXPOSE 7979
ENTRYPOINT ["marqueue"]
