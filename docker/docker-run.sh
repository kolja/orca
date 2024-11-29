#!/bin/bash

docker run -p 8080:8080 \
-e ORCA_CONFIG=/app/docker.orca.toml \
-v $(pwd)/docker.orca.toml:/app/docker.orca.toml \
-v $(pwd)/templates:/app/templates \
-v /Volumes/library:/app/library \
-v /Volumes/nonfiction:/app/nonfiction \
koljaw/orca:latest
