# workspace_fs

ローカルディレクトリを repository として扱う、Rust 製の安全境界つき file server。
HTTP リクエスト経由でディレクトリの編集を行う。

## 概要
- 起動時引数で repository root となる path を受け取る。
- repository root 外は触らない（ validation や sanitize を行う）。
- `REPOSITORY/.repo/` 以下は専用のディレクトリとする。
- config で細かい指定ができる。
- plugin で hook を書いて生成などをすることができるようにする。
- task で起動前に plugin invoke ができるようにする。

> [!warning]
> このサーバーではユーザーの認証・https 化は行わない。
> 必要があれば wrapper を通すこと。

[[テスト]]

## 起動方法

```bash
cargo run --bin workspace_fs -- ./test-repository
```

起動時には読み込んだ serve 設定をログに出す。
また、各リクエストについて method, path, status code をログに出す。

起動後の例:

- `GET /`:
  - repository root 直下の一覧
- `GET /docs/`:
  - `docs` 直下の一覧
- `POST /notes/`:
  - `notes` ディレクトリを作成
- `GET /index.md`:
  - `index.md` の本文
- `PUT /index.md`:
  - 既存の `index.md` を上書き保存
- `POST /new.md`:
  - `new.md` を新規作成
- `DELETE /notes/`:
  - 空の `notes` ディレクトリを削除
- `DELETE /new.md`:
  - `new.md` を削除

## API
URL をほとんどそのままパスとみて、ファイルへの編集が行えるようにする。

- `/PATH/` はディレクトリに対応し、`/FILE` はファイルに対応する。
- `GET URL` は内容の取得
  - `GET /dir/` なら、 ディレクトリ直下の内容を 1 entry 1 line で返す。
  - `GET /file.txt` ならファイルの内容をそのまま返す。
- `POST URL` は新規作成
  - `POST /dir/` ならディレクトリを新規作成する。
  - `POST /file.txt` ならファイルを新規作成する。
  - いずれにせよ、すでに存在していたらエラーとする。
- `PUT /file.txt` で既存ファイルを更新する。
  - 存在しない場合はエラーとする。
- `DELETE URL` は削除。
  - `DELETE /dir/` ならディレクトリを削除する、**ただし、空のディレクトリのときだけ。**
  - `DELETE /file.txt` ならファイルを削除する。

> [!warning]
> 柔軟な対応はしない。愚直に対応する。
> - URL は HTTP リクエストでは `/` で始まる絶対パスの記述だが、ディレクトリは相対パス（ `/` で始まったらエラー）。
> - 途中のパスが存在しない場合は（新規作成等を含めて）エラーとする。
> - `/` が最後につくかどうかでディレクトリとファイルどっちへの要求なのかを区別する。
> - ファイルでの指定でディレクトリが見つかったときは、ディレクトリに直さずにエラーにする。

> [!warning]
> URL としては `/.repo/` は一切指定できないものとする。
> また、 mount を経由していようといまいと、 `/.repo/` 以下の書き換えになる行為は API 経由は不可とする。
> 閲覧については設定次第。

また、 `.repo/` 以外で予約されている path の prefix があるが、設定で変えられる。（ `.repo/` は変えられない。）
以下はデフォルトの prefix
- plugin を走らせる： `.plugin/`
- policy を確認する： `.policy/`
- 基本情報を確認する： `.info/`

### user-identity について
GET を除いたリクエストで `user-identity` （文字列）を設定すること。
> [!warning]
> `POST` / `PUT` / `DELETE` で `user-identity` が設定されていないならリクエストを拒否する。

### レスポンスの内容について
`file.html` なら html と書かなければいけないし、 wasm も配信したいので、次のようになっている。
- HTTP body はファイルの場合は全て `[u8]` が返ってくると思うこと。
- ヘッダでは MIME をファイルの拡張子から推測し、 `Content-Type` を設定する。

## config
`REPOSITORY/.repo/config.toml` で設定を書く。

### name
repository の名前を指定する。必須。
```
name = "computation"
```

- plugin に対して `REPOSITORY_NAME` として渡される。

### serve
port や url prefix の指定をする。
```
[serve]
port = 3030
plugin_url_prefix = "/.plugin"
policy_url_prefix = "/.policy"
info_url_prefix = "/.info"
```
- `plugin_url_prefix` は manual plugin 実行 API の prefix
- `policy_url_prefix` は policy 診断 API の prefix
- `info_url_prefix` は path 基本情報 API の prefix

> [!warning]
> prefix の最後には `/` を付けないこと。
> （ディレクトリではあるが、組み立て方の都合上）

### policy
path に対して API 経由での GET/POST/DELETE/PUT をやってよいかを指定できる。
```
[[policy]]
path = ".git/"
GET = false
POST = false
PUT = false
DELETE = false
```
なお、 `.repo/` 以下は設定できない。

### plugin
hook のような形で、プラグインを記述する。内部では `{PLACE_HOLDER}` の記法が使える。
```
[[plugin]]
name = "convert-md-html"
runner = "command"
deps = ["link"]
command = ["python3", "./convert-md-html.py", "{GET.PATH}"]
trigger = "GET"
path = "A.md"
mount = "/assets/"
```
上のプラグインは外部コマンドの invoke を行う：
`A.md` に該当する GET があったときに `{GET.PATH}` をファイル名で置き換えて実行する。
また、 `.repo/convert-md-html/generated/` を `/assets/` に mount する。

deps はこの plugin が依存している他の plugin の指定である。

- `mount` は省略可能
- `mount` を指定した場合、 plugin の出力先 `/.repo/<PLUGIN_NAME>/generated/` がその URL prefix `/<PLUGIN_NAME>/` に公開される
- mount は **GET のみ**
- `mount` は `/` で始まり（URL としての絶対パス指定） `/` で終わること（ディレクトリであることの明示）

> [!warning]
> `mount` がすでに `REPOSITORY/` に存在するディレクトリとかぶったらエラーとする。
> 同じ `mount` を複数 plugin で使うことも不可。

> [!note]
> 将来的には、 `"command"` じゃなくて wasm も指定できるとうれしいが、 interface を考えるのが難しい。

また、 plugin 固有の設定値も書ける。
```
[[plugin]]
name = "md_preview"

[plugin.md_preview]
enhance = true
```
とか

### task
plugin をどの順番に実行するかを書いて、起動時に指定する。
```
[[task]]
name = "build"
steps = ["build-wasm", "build-autosummary"]
```

## policy
config.toml 以外での設定は受け付けない。

> [!warning]
> `[policy]` で指定された path 以外は一切公開しない。
> `path` は必須、それ以外は指定しなくてもいい。
> 指定しなかった場合： `GET` は `true` `POST/PUT/DELETE` は `false` とする。

### 複数の policy に match する場合の優先度について
glob を許さないので、あるファイルやディレクトリにマッチする場合の具体度が同じになることはなさそう。
現在の path の policy について調べたいときは後述の API を使うこと。

### policy 診断 API
`GET <policy_url_prefix>/PATH` で、その path に対して
- match した policy 一覧
- 実際に採用された policy
- 最終的な GET/POST/PUT/DELETE の有効値

を JSON で返す。

例：
```json
{
  "path": "docs/private/a.md",
  "matches": [
    {
      "index": 0,
      "pattern": "docs/",
      "specificity": { "depth": 1, "chars": 4 },
      "permissions": { "GET": true, "POST": true, "PUT": true, "DELETE": true }
    }
  ],
  "selected": {
    "index": 0,
    "pattern": "docs/",
    "reason": "more_specific"
  },
  "effective": { "GET": true, "POST": true, "PUT": true, "DELETE": true }
}
```

この API は誰でも `GET` できる。

## path info
`GET <info_url_prefix>/PATH` で、その path に対して
- path
- kind (`file` または `directory`)
- size
- modified_at
- readonly

を JSON で返す。

例：
```json
{
  "path": "docs/private/a.md",
  "kind": "file",
  "size": 1234,
  "modified_at": "2026-03-19T12:34:56Z",
  "readonly": false
}
```

- ディレクトリでは `size` は `null`
- `modified_at` が取得できない環境では `null`
- 通常の path API と同様に、ディレクトリを指定するときは末尾に `/` が必要
- この API は誰でも `GET` できるが、対象 path の `GET` policy は適用される


## plugin
`config.toml` で指定したもののみを対象とする。
名前は以下のもののみが許可される：`[A-Za-z_][A-Za-z0-9_\-]*`

### deps
存在している plugin のみ指定出来て、
循環参照はエラーとする。
これを記述すると、 deps に追加した他の plugin の公開 URL が得られる。

### 実行タイミングと trigger
実行するタイミングは、
1. `trigger = "manual"` 以外の場合は、特定の API 操作が呼ばれたとき。
2. `trigger = "manual"` の場合には、
  - `POST <plugin_url_prefix>/<PLUGIN_NAME>/run` が来た時
  - `task` で指定されたとき... serve の前に行われる。

> [!warning]
> plugin が書き換えるのは
> - `.repo/<PLUGIN_NAME>/generated/` ... mount で使える、 API で露出するようのディレクトリ：最終成果物など
> - `.repo/<PLUGIN_NAME>/cache/` ... API 経由では触れないディレクトリ：中間成果物やキャッシュなど
> それ以外の書き換えは自己責任とする。

例：
```
[[plugin]]
name = "wasm-build"
runner = "command"
deps = ["delete-by-gitignore"]
command = ["cargo", "build", "--target", "wasm32-unknown-unknown"]
trigger = "manual"
```
これは明らかに `REPOSITORY_ROOT/target/` を書き換えるが、無視する。
同様に、 git を使って自動で履歴保存とかも同じようになるはず。

### place holder について
基本的には trigger ごと設定できる項目を分けて、ここに乗っているもの以外は評価をしない。

全体で使えるもの
- `REPOSITORY_NAME` ... `config.toml` の `name`
- `REPOSITORY_ROOT` ... このリポジトリの絶対パス
- `PLUGIN_NAME` ... plugin に設定された名前
- `OUTPOST_DIRECTORY` ... 各 plugin が書き込んでよいパス `.repo/<PLUGIN_NAME>/generated/` のこと
- `MOUNT_URL` ... `config.toml` で設定されている mount 先の url prefix
- `MOUNT_{OTHER_PLUGIN_NAME}`
  - `deps = [...]` で指定されたものに限り、その plugin の `MOUNT_URL` を得ることができる。
  - ただし、環境変数に合わせるため、 plugin の名前は `-` は `_` に、 英小文字は英大文字にしたうえで、 `[A-Z_][A-Z0-9_]*` に強制される
  - 例： `deps = ["build-WASM_test"]` なら `MOUNT_BUILD_WASM_TEST` でアクセスできるようになる。

GET
- `GET.PATH`
- `GET.USER-IDENTITY`

POST/PUT/DELETE も同様のものだけ実装する。

## task
task が指定されたときに、 serve 前に指定された plugin を順番に起動する。
task は plugin の順序のみを指定する。それ以外は特にない。

# 実装について

## Rust の責務分割

- HTTP 層はルーティングとプレーンテキスト入出力だけを担当する
- repository のパス解決、一覧、読込、作成、更新、削除は `Repository` trait の実装に閉じ込める
- `config.toml` の読込は `config` module の専用構造体で扱う
- wrapper/proxy から渡された user identity の取込みは `identity` module で扱う
- 現在はファイルシステム実装として `FsRepository` を使う
- 将来的に別実装を足しても、HTTP 層は trait 越しに扱う

## Identity

- この server 自体は認証しない
- 外部 wrapper/proxy が認証済みユーザーをヘッダで渡す前提にする
- 現在は request ごとに user identity を `String` として request context に積むだけにする
- `GET` は `user-identity` なしでもよく、その場合は空文字列として扱う
- user identity のヘッダ名は `user-identity` に固定する

例:

```http
user-identity: alice
```

## パス安全性

- `..` を含む path は拒否する
- 絶対パスは拒否する
- `.repo/` 配下は API から直接触れない
- 保存時も repository 相対パスを正規化してから処理する

## 今後の拡張

- plugin / hook system
- 履歴管理 plugin
- git backend plugin
- wasm component による安全な拡張実行
