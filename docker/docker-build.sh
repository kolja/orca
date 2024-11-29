#!/bin/bash

DOCKERFILE="${CARGO_MAKE_WORKING_DIRECTORY}/docker/Dockerfile"
IMAGE_NAME=${1}
CARGO_TOML_PATH="${CARGO_MAKE_WORKING_DIRECTORY}/Cargo.toml"

# need Cargo.toml to extract the version
if [ ! -f $CARGO_TOML_PATH ]; then
    echo "Error: Cargo.toml not found!"
    exit 1
fi

# an IMAGE_NAME is required as an argument.
# e.g. "koljaw/orca" it shouldn't contain a tag or version
if [ -z $IMAGE_NAME ]; then
    echo "Please provide a name for your Docker image"
    echo "Like so: \`cargo make docker-build foo/bar\`"
    exit 1
fi

VERSION=$(awk -F ' = ' '$1 ~ /version/ { gsub(/[\"]/, "", $2); printf("%s",$2) }' $CARGO_TOML_PATH)

# Create and use the Buildx builder if it doesn't exist
docker buildx inspect mybuilder > /dev/null 2>&1
if [ $? -ne 0 ]; then
    docker buildx create --name mybuilder --use
    docker buildx inspect --bootstrap
else
    docker buildx use mybuilder
fi

# Build and push the image
docker buildx build -f ${DOCKERFILE} --platform linux/amd64,linux/arm64 --build-arg VERSION=v${VERSION} -t ${IMAGE_NAME}:v${VERSION} --push .
