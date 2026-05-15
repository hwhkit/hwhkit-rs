# cargo-hwhkit

`cargo` subcommand for the [`hwhkit`](https://crates.io/crates/hwhkit)
toolkit.

## Install

```sh
cargo install cargo-hwhkit
```

## Commands

- `cargo hwhkit init [--template minimal-api]` — scaffold a new
  service from a template (config/, src/, default features wired).
- `cargo hwhkit migrate {create,list,run,revert}` — manage `sqlx`
  migrations under the path declared in
  `[integrations.sql.postgres.migrations]`.
- `cargo hwhkit dev` — bring up the local dependency stack (Postgres,
  Redis, …) via docker-compose for development.

Run `cargo hwhkit --help` for the full command set.

## License

MIT OR Apache-2.0
