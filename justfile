default: build

build:
    cargo build --features=fundsp --release --example=*

test:
    cargo test

clippy:
    cargo clippy --features=fundsp --example=*

web-build:
    cd examples/play-emscripten && ./build.sh

web-serve:
    cd examples/play-emscripten && ./serve.sh

web-run: web-build web-serve
