#!/bin/bash

DOCKERFILE="${CARGO_MAKE_WORKING_DIRECTORY}/docker/Dockerfile.demo"
CARGO_TOML_PATH="${CARGO_MAKE_WORKING_DIRECTORY}/Cargo.toml"

# Takes the *release* image name, the same argument `docker-build` takes, and
# builds `<name>-demo` on top of it.
BASE_IMAGE=${1}
IMAGE_NAME="${1}-demo"

if [ ! -f $CARGO_TOML_PATH ]; then
    echo "Error: Cargo.toml not found!"
    exit 1
fi

if [ -z $BASE_IMAGE ]; then
    echo "Please provide the name of the image the demo is built on"
    echo "Like so: \`cargo make docker-build-demo koljaw/orca\`"
    exit 1
fi

# The argument is the release image, not the demo: `koljaw/orca-demo` would
# otherwise be published as `koljaw/orca-demo-demo`.
case "$BASE_IMAGE" in
    *-demo)
        echo "Error: pass the release image, not the demo: \`${BASE_IMAGE%-demo}\`"
        exit 1
        ;;
esac

VERSION=$(awk -F ' = ' '$1 ~ /version/ { gsub(/"/, "", $2); printf("%s",$2) }' $CARGO_TOML_PATH)

# The release-image has to exist before we can build the demo
docker manifest inspect ${BASE_IMAGE}:v${VERSION} > /dev/null 2>&1
if [ $? -ne 0 ]; then
    echo "Error: ${BASE_IMAGE}:v${VERSION} is not on the registry."
    echo "Run \`cargo make docker-build ${BASE_IMAGE}\` first."
    exit 1
fi

docker buildx inspect mybuilder > /dev/null 2>&1
if [ $? -ne 0 ]; then
    docker buildx create --name mybuilder --use
    docker buildx inspect --bootstrap
else
    docker buildx use mybuilder
fi

CMD="docker buildx build -f ${DOCKERFILE} --platform linux/amd64,linux/arm64 --build-arg VERSION=v${VERSION} --build-arg BASE=${BASE_IMAGE} -t ${IMAGE_NAME}:v${VERSION} -t ${IMAGE_NAME}:latest --push ."
echo -e "Running Now:\n $CMD"
$CMD
