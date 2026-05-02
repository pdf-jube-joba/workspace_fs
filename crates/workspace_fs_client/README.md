# workspace_fs_client

テスト

`workspace_fs_client` は local HTTP proxy と task runner です。`REPOSITORY/.repo/user.toml` を読み、`repository.port` ごとに browser 向けの入口を立てます。`mode = "spawn"` の repository は必要に応じて local server も起動します。path を省略すると cwd を使います。

## 起動

workspace root から:

```bash
cargo run -- ./test-repository
```

path を省略して cwd を読むこともできます。

client crate を直接起動する場合:

```bash
cargo run -p workspace_fs_client -- ./test-repository
```

引数:

- `[repository-path]`: `.repo/user.toml` を読む対象 repository。省略すると cwd

## user.toml

client 側の設定は `REPOSITORY/.repo/user.toml` に書きます。

```toml
[[repository]]
name = "local"
mode = "spawn"
port = 3031
as = "alice_browser"

[repository.server]
port = 3000

[[task]]
name = "build"

[[task.step]]
repository = "local"
plugin = "md-preview"
```

- `repository.name`: client 側で使う接続先名
- `repository.mode`: `spawn` か `attach`
- `repository.port`: browser から見える client proxy の待受 port
- `repository.as`: upstream へ付与する `user-identity`
- `repository.where`: `attach` のときの HTTP 接続先。`spawn` のときは省略する
- `repository.plugin_url_prefix`: plugin 実行 API の prefix。attach のときに使う
- `repository.server.port`: `spawn` する server の待受 port
- `repository.server.plugin_url_prefix`: spawn する server の plugin prefix
- `repository.server.policy_url_prefix`: spawn する server の policy prefix
- `repository.server.info_url_prefix`: spawn する server の info prefix
- `repository.server.args`: spawn する server に渡す追加引数
- `task.name`: task の名前
- `task.step.repository`: どの repository に投げるか
- `task.step.plugin`: 実行する plugin 名

## 動作

- browser や CLI は repository ごとの client proxy にアクセスする
- client は受けた request を upstream server へ転送する
- 転送時に `user-identity` を上書き注入する
- `path` を省略すると cwd の `.repo/user.toml` を読む
- `mode = "spawn"` の repository は client が local server を spawn する
- `mode = "attach"` の repository は `where` へ HTTP で接続する
- `https://` は未対応で、`where` は HTTP only
- task は `POST <plugin_url_prefix>/<plugin>/run` を順に叩く
- 通常起動時に REPL も開き、`task <task-name>` と `plugin <repository-name> <plugin-name>` を line by line で受け付ける

## 注意

- `user.toml` に `[[repository]]` が 1 つもない場合は起動しない
- `repository.port` は repository ごとに重複できない
- `mode = "spawn"` のとき `repository.server.port` は repository ごとに重複できない
- `mode = "attach"` のとき `repository.where` は必須
- `mode = "spawn"` のとき `repository.where` と `repository.server` は書けない
