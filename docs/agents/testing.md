# Testing

Run the tests of the whole Rust workspace from the repo root:

```sh
cargo test --workspace
```

The `ups` crate builds only on Linux. On another system, add `--exclude ups`
and do not mention it to the Admin: CI runs it on Linux.
