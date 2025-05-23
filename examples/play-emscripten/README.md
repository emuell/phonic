# Emscripten Demo

This example crate shows how to build and serve a simple website which is using phonic via Emscripten.

It creates a webpage which plays a metronome sample file and allows playing a simple synth sound with the computer keyboard.


## Build Instructions

To build phonic for the web with emscripten you need to install:

### Prerequisites

- Install [Emscripten SDK](https://emscripten.org/docs/getting_started/downloads.html)
- Add wasm32-unknown-emscripten target for rust: `rustup target add wasm32-unknown-emscripten`
- Add rust-src for cargo build-std: `rustup component add rust-src`

> [!NOTE]
> `cargo +nightly -Z build-std` unfortunately is needed to get the example compiled with **pthread** support. When building the wasm without pthread support, this won't be necessary.

See [./build.sh](./build.sh) file for details. 

### Build 

Then use the build script in the example's root folder:

```bash
./build.sh
```


## Run Instructions

### Prerequisites

- Install simple-http-server or some other lightweight http server: `cargo [b]install simple-http-server`

### Run

```bash
./serve.sh
```

Then open a web browser at http://localhost:8000

> [!NOTE]
> **Cross-Origin-Embedder-Policy** and **Cross-Origin-Opener-Policy** headers are necessary for pthread (web-workers) support. When building the wasm without pthread support, this won't be necessary.

See [./serve.sh](./serve.sh) for details.
