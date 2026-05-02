# workspace_fs

`workspace_fs` はディレクトリを HTTP で操作できるようにするソフトです。

- server: [crates/workspace_fs_server/README.md](/home/namaniyu/source/workspace_fs/crates/workspace_fs_server/README.md)
  - `.repo/config.toml` を読んで、対象 repository に対する API を作ります。
- client: [crates/workspace_fs_client/README.md](/home/namaniyu/source/workspace_fs/crates/workspace_fs_client/README.md)
  - `REPOSITORY/.repo/user.toml` を読み、`repository.port` ごとに browser 向けの入口を立てます。

主な起動例:

```bash
cargo run -- ./test-repository
```

これは root package `workspace_fs` を起動し、`cwd` または指定 path の `.repo/user.toml` を読んで client を立てます。`mode = "spawn"` の repository は local server も起動します。

対話モードで起動する場合は `--repl` を付けます。

```bash
cargo run -- ./test-repository --repl
```

server を直接起動する場合は次です。

```bash
cargo run -p workspace_fs_server -- ./test-repository
```
