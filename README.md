<h1 align="center">
    tower-sessions
</h1>

<p align="center">
    User sessions as a `tower` middleware.
</p>

<div align="center">
    <a href="https://crates.io/crates/tower-sessions">
        <img src="https://img.shields.io/crates/v/tower-sessions.svg" />
    </a>
    <a href="https://docs.rs/tower-sessions">
        <img src="https://docs.rs/tower-sessions/badge.svg" />
    </a>
    <a href="https://github.com/maxcountryman/tower-sessions/actions/workflows/rust.yml">
        <img src="https://github.com/maxcountryman/tower-sessions/actions/workflows/rust.yml/badge.svg" />
    </a>
    <a href="https://codecov.io/gh/maxcountryman/tower-sessions" > 
        <img src="https://codecov.io/gh/maxcountryman/tower-sessions/graph/badge.svg?token=74POF0TJDN"/> 
    </a>
</div>

## TODOs
- [ ] tracing.
- [ ] TEST


## Session stores

Session data persistence is managed by user-provided types that implement
`SessionStore`. What this means is that applications can and should
implement session stores to fit their specific needs.

That said, a number of session store implmentations already exist and may be
useful starting points.

| Crate                                                                                                            | Persistent | Description                                                 |
| ---------------------------------------------------------------------------------------------------------------- | ---------- | ----------------------------------------------------------- |
| [`tower-sessions-redis-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/redis-store)     | Yes        | Redis via `fred` session store                              |

Have a store to add? Please open a PR adding it.

## Usage

To use the crate in your project, add the following to your `Cargo.toml` file:

```toml
[dependencies]
tower-sessions = "0.13.0"
```
You can find this [example][counter-example] as well as other example projects in the [example directory][examples].

> [!NOTE]
> See the [crate documentation][docs] for more usage information.

## Getting Help

We've put together a number of [examples][examples] to help get you started. You're also welcome to [open a discussion](https://github.com/maxcountryman/tower-sessions/discussions/new?category=q-a) and ask additional questions you might have.

## 👯 Contributing

We appreciate all kinds of contributions, thank you!

[counter-example]: https://github.com/maxcountryman/tower-sessions/tree/main/examples/counter.rs
[examples]: https://github.com/maxcountryman/tower-sessions/tree/main/examples
[docs]: https://docs.rs/tower-sessions
