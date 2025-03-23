# smoothmirror
High framerate remote desktop software for sharing games or remote productivity

## Features
- High framerate, low latency
- Opus encoded audio
- Multiplatform: Capture on Windows & Linux. Display on Windows, Mac & Linux
- Forward keyboards, mice, game controllers, even USB ports!
- Hardware accelerated encode using NVENC with Nvidia cards (use `nvenc` feature)

## Usage
Run the repeater on a server with ports 42069/tcp and 42069/udp publicly accessible.
```
cargo run --bin repeater
```

Then run the display and capture clients on the repective computers
```
cargo run --bin display
```
```
cargo run --bin capture
```

When the display client connects and starts, use F7 to close the UI and control the remote computer.
