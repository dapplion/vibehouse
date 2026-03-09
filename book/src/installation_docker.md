# Docker Guide

There are two ways to obtain a Vibehouse Docker image:

1. [Docker Hub](#docker-hub), or
2. By [building a Docker image from source](#building-the-docker-image).

Once you have obtained the docker image via one of these methods, proceed to [Using the Docker
image](#using-the-docker-image).

## Docker Hub

Vibehouse maintains the [dapplion/vibehouse][docker_hub] Docker Hub repository which provides an easy
way to run Vibehouse without building the image yourself.

Obtain the latest image with:

```bash
docker pull dapplion/vibehouse
```

Download and test the image with:

```bash
docker run dapplion/vibehouse vibehouse --version
```

If you can see the latest [Vibehouse release](https://github.com/dapplion/vibehouse/releases) version
(see example below), then you've successfully installed Vibehouse via Docker.

### Example Version Output

```text
Vibehouse vx.x.xx-xxxxxxxxx
BLS Library: xxxx-xxxxxxx
```

### Available Docker Images

There are several images available on Docker Hub.

Most users should use the `latest` tag, which corresponds to the latest stable release of
Vibehouse with optimizations enabled.

To install a specific tag (in this case `latest`), add the tag name to your `docker` commands:

```bash
docker pull dapplion/vibehouse:latest
```

Image tags follow this format:

```text
${version}${arch}${stability}
```

The `version` is:

* `vX.Y.Z` for a tagged Vibehouse release, e.g. `v2.1.1`
* `latest` for the `stable` branch (latest release) or `unstable` branch

The `arch` is:

* `-amd64` for x86_64, e.g. Intel, AMD
* `-arm64` for aarch64, e.g. Raspberry Pi 4
* empty for a multi-arch image (works on either `amd64` or `arm64` platforms)

The `stability` is:

* `-unstable` for the `unstable` branch
* empty for a tagged release or the `stable` branch

Examples:

* `latest-unstable`: most recent `unstable` build
* `latest-amd64`: most recent Vibehouse release for older x86_64 CPUs
* `latest-amd64-unstable`: most recent `unstable` build for older x86_64 CPUs

## Building the Docker Image

To build the image from source, navigate to
the root of the repository and run:

```bash
docker build . -t vibehouse:local
```

The build will likely take several minutes. Once it's built, test it with:

```bash
docker run vibehouse:local vibehouse --help
```

## Using the Docker image

You can run a Docker beacon node with the following command:

```bash
docker run -p 9000:9000/tcp -p 9000:9000/udp -p 9001:9001/udp -p 127.0.0.1:5052:5052 -v $HOME/.vibehouse:/root/.vibehouse dapplion/vibehouse vibehouse --network mainnet beacon --http --http-address 0.0.0.0
```

> To join the Hoodi testnet, use `--network hoodi` instead.

> The `-v` (Volumes) and `-p` (Ports) and values are described below.

### Volumes

Vibehouse uses the `/root/.vibehouse` directory inside the Docker image to
store the configuration, database and validator keys. Users will generally want
to create a bind-mount volume to ensure this directory persists between `docker
run` commands.

The following example runs a beacon node with the data directory
mapped to the users home directory:

```bash
docker run -v $HOME/.vibehouse:/root/.vibehouse dapplion/vibehouse vibehouse beacon
```

### Ports

In order to be a good peer and serve other peers you should expose port `9000` for both TCP and UDP, and port `9001` for UDP.
Use the `-p` flag to do this:

```bash
docker run -p 9000:9000/tcp -p 9000:9000/udp -p 9001:9001/udp dapplion/vibehouse vibehouse beacon
```

If you use the `--http` flag you may also want to expose the HTTP port with `-p
127.0.0.1:5052:5052`.

```bash
docker run -p 9000:9000/tcp -p 9000:9000/udp -p 9001:9001/udp -p 127.0.0.1:5052:5052 dapplion/vibehouse vibehouse beacon --http --http-address 0.0.0.0
```

[docker_hub]: https://hub.docker.com/repository/docker/dapplion/vibehouse/
