# speed

An interactive, playful [Fast.com](https://fast.com) speed test for your terminal. It runs once when opened, then stays quiet until you press <kbd>Space</kbd> or <kbd>Enter</kbd> for another lap.

```text
 SPEED // FAST.COM
 ●  Ready for another lap

              287.4
          flying — 4K has room to spare

    ⣀⣀⡀      ⢀⣤⣶⣿⣿⣶⣤⣀
       ⠉⠛⠿⠿⠛⠉

 SPACE/ENTER rerun   Q quit
```

## Install

Install the current stable Rust toolchain first.

```sh
cargo install --git https://github.com/realzai/speed
```

Or build it locally:

```sh
git clone https://github.com/realzai/speed.git
cd speed
cargo install --path .
```

## Use

```sh
speed          # interactive UI
speed --once   # one-shot output for scripts and pipes
speed --help
```

Inside the interactive UI:

- <kbd>Space</kbd>, <kbd>Enter</kbd>, or <kbd>R</kbd> starts another test.
- <kbd>Q</kbd>, <kbd>Esc</kbd>, or <kbd>Ctrl+C</kbd> exits.

Each run measures unloaded latency and download throughput against nearby Netflix Open Connect servers. Six parallel downloads run for ten seconds to give the connection time to reach full speed. No test runs in the background while the app is waiting.

## Notes

This is an unofficial client and is not affiliated with Netflix. Results can differ slightly from the browser version because browsers, Wi-Fi conditions, VPNs, and concurrent traffic all affect measurement.

## License

MIT
