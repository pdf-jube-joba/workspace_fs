# workspace_fs_server

`workspace_fs_server` は repository に対する HTTP file server です。認証機構は持たず、各 request の `user-identity` を前提に policy と plugin 実行権限を判断します。

## 起動

workspace root から:

```bash
cargo run -p workspace_fs_server -- ./test-repository
```

起動時には `REPOSITORY/.repo/config.toml` を読み込みます。
`[serve]` の各値は CLI で上書きできます。

```bash
cargo run -p workspace_fs_server -- ./test-repository --port=4040 --plugin-url-prefix=/.plugin2
```

現在上書きできる値:

- `--port=...`
- `--plugin-url-prefix=...`
- `--policy-url-prefix=...`
- `--info-url-prefix=...`

## API

- `GET /`: repository root の一覧
- `GET /PATH/`: ディレクトリ一覧
- `GET /FILE`: ファイル内容
- `POST /PATH/`: ディレクトリ作成
- `POST /FILE`: ファイル新規作成
- `PUT /FILE`: 既存ファイル更新
- `DELETE /PATH/`: 空ディレクトリ削除
- `DELETE /FILE`: ファイル削除
- `POST <plugin_url_prefix>/<plugin>/run`: plugin 実行
- `GET <policy_url_prefix>/PATH`: policy 診断
- `GET <info_url_prefix>/PATH`: path info

## user-identity

すべての request に `user-identity` ヘッダが必要です。

```http
user-identity: alice_browser
```

この server 自体は認証しません。認証は必要に応じて wrapper を書いてください。
このリポジトリの client でも認証を行っていません。

## config.toml

server 側の設定は `REPOSITORY/.repo/config.toml` に書きます。

### name

```toml
name = "computation"
```

### serve

```toml
[serve]
port = 3030
plugin_url_prefix = "/.plugin"
policy_url_prefix = "/.policy"
info_url_prefix = "/.info"
```

- `plugin_url_prefix`: plugin 実行 API の prefix
- `policy_url_prefix`: policy 診断 API の prefix
- `info_url_prefix`: path info API の prefix

prefix は `/` で始まり、末尾の `/` は付けません。
CLI からは `--{config-name}=...` で上書きできます。例えば `plugin_url_prefix` は `--plugin-url-prefix=/.plugin2` です。

### policy

path ごとに method 単位の whitelist を設定します。

```toml
[[policy]]
path = "docs/"
GET = ["alice_browser", "alice_cli"]
POST = ["alice_cli"]
PUT = ["alice_cli"]
DELETE = []
```

- `path` は repository 相対
- `.repo/` 以下は指定不可
- 指定しなかった method は空配列
- policy に match しない path は deny

複数 rule が match した場合は、より具体的な path が優先され、同じ具体度なら後勝ちです。

### ignore

```toml
[ignore]
paths = [".git", "LICENSE"]
```

ignore された path は一覧から隠され、直接アクセスも拒否されます。

### plugin

plugin は HTTP 経由で明示的に実行します。

```toml
[[plugin]]
name = "convert-md-html"
runner = "command"
command = ["python3", "./convert-md-html.py"]
allow = ["alice_browser"]
mount = "/assets/"
```

- `name`: `[A-Za-z_][A-Za-z0-9_-]*` のみ
- `runner`: `command` または `default`
- `command`: `runner = "command"` のとき必須
- `allow`: 実行可能な `user-identity` 一覧
- `mount`: 省略可。`/.repo/<plugin>/generated/` を URL に mount

`mount` がある plugin は、その mount path に対して暗黙の `GET` policy が入り、`allow` と同じ identity だけ読めます。

`runner` が `default` になっている場合は、 **この** リポジトリにある `default.toml` で指定された方法を使います。

```toml
[[plugin]]
name = "md-preview"
runner = "default"
allow = ["alice_browser"]
mount = "/md/"

[plugin.md_preview]
enhance = []
```

## plugin 実行時の前提

- cwd は repository root
- 出力先ディレクトリ:
  - `.repo/<PLUGIN_NAME>/generated/`
  - `.repo/<PLUGIN_NAME>/cache/`
- それ以外の書き換えは実装上は止めていないが、自己責任

plugin には主に次の環境変数を渡します。

- `WORKSPACE_FS_REPOSITORY_ROOT`
- `WORKSPACE_FS_REPOSITORY_NAME`
- `WORKSPACE_FS_PLUGIN_NAME`
- `WORKSPACE_FS_OUTPUT_DIRECTORY`
- `WORKSPACE_FS_CACHE_DIRECTORY`
- `WORKSPACE_FS_PLUGIN_SETTINGS_JSON`
- `WORKSPACE_FS_USER_IDENTITY`
- `MOUNT_URL` (`mount` がある場合)

placeholder で使える値:

- `{REPOSITORY_NAME}`
- `{REPOSITORY_ROOT}`
- `{PLUGIN_NAME}`
- `{OUTPOST_DIRECTORY}`
- `{OUTPUT_DIRECTORY}`
- `{WORKSPACE_FS_ROOT}`
- `{DEFAULT_PLUGINS_ROOT}`
- `{MOUNT_URL}`

## path safety

- `..` を含む path は拒否
- 絶対 path は拒否
- `.repo/` 配下は HTTP API から直接触れない
- 保存時も repository 相対 path を正規化してから処理する
