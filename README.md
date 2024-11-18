# tower-sesh
An opinionated session middleware for `tower` services.

## Comparison with `tower-sessions`
`tower-sessions` tries to follow the design of `django`'s session middleware. As a consequence,
every request does the following:
- allocate multiple `HashMap`s,
- Use dynamic dispatch for futures (using `Pin<Box<dyn Future>>`)
- make extensive use of the `Arc<Mutex<_>>` magic sauce.

We don't do that here.

## Session stores
Session data persistence is managed by user-provided types that implement
`SessionStore`. What this means is that applications can and should
implement session stores to fit their specific needs.

That said, a number of session store implementations already exist and may be
useful starting points.

| Crate                                                                            | Persistent | Description               |
| ---------------------------------------------------------------------------------| ---------- | ------------------------- |
| [`tower-sesh-redis-store`](https://github.com/carloskiki/tower-sesh-redis-store) | Yes        | Redis using `redis` crate |

Have a store to add? Please open a PR adding it.

## Usage

This crate is not published on crates.io. You need to add it as a git dependency.
```toml
[dependencies]
tower-sesh = { git = "https://github.com/carloskiki/tower-sesh.git" }
```

## Contributing

All contributions are welcome! All are licensed under the MIT license.
