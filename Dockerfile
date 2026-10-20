# A reproducible OQCI environment (final-deliverables-spec.md §27): the pinned
# Rust toolchain, Python 3.12, the compiler, the Python SDK with every
# execution adapter's dependencies, and the benchmark harness.
#
#   docker build -t oqci --build-arg OQCI_GIT_COMMIT=$(git rev-parse HEAD) .
#   docker run --rm oqci cargo test --workspace --exclude oqci-python
#   docker run --rm oqci python -m pytest python/tests -q
#   docker run --rm -v "$PWD/results:/oqci/benchmarks/results" oqci \
#       sh -c "cd benchmarks && python -m oqci_bench run --config configs/smoke.toml"
#
# No credentials are baked in. For IBM hardware, mount a saved account at
# run time (-v ~/.qiskit:/root/.qiskit:ro); never copy one into an image.

FROM python:3.12-slim-bookworm

ARG RUST_TOOLCHAIN=1.96.0
# The image has no .git; provenance records this commit, or "unknown".
ARG OQCI_GIT_COMMIT=unknown
ENV OQCI_GIT_COMMIT_OVERRIDE=${OQCI_GIT_COMMIT}

RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates build-essential git llvm-14 \
 && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain ${RUST_TOOLCHAIN} --component rustfmt,clippy

WORKDIR /oqci
COPY . .

RUN cargo build --release --bin oqci \
 && pip install --no-cache-dir -r python/requirements-dev.txt -r python/requirements-qir.txt \
        -r benchmarks/requirements.txt \
 && pip install --no-cache-dir ./python

ENV PATH=/oqci/target/release:$PATH
CMD ["oqci", "--help"]
