FROM ubuntu:latest

WORKDIR /app

RUN apt-get update && apt-get install -y curl && apt-get clean

RUN rm -f orca-x86_64-unknown-linux-musl.tar.gz

ARG VERSION
RUN curl -L -o orca-x86_64-unknown-linux-musl.tar.gz https://github.com/kolja/orca/releases/download/${VERSION}/orca-x86_64-unknown-linux-musl.tar.gz

RUN tar -xzf orca-x86_64-unknown-linux-musl.tar.gz

RUN rm orca-x86_64-unknown-linux-musl.tar.gz

RUN chmod 755 /app/orca

CMD ["./orca"]
