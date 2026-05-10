# md_preview

`md_preview` は markdown preview と directory browser 用の静的 asset を生成する default plugin です。`build.mjs` が `src/` 以下を bundle し、viewer HTML と script、KaTeX 関連 asset、link index を出力します。

## build

plugin root で:

```bash
node ./build.mjs --out-dir <path>
```

引数:

- `--out-dir <path>`: 生成物の出力先。必須
- `--repository-root <path>`: markdown 走査や `macro_path` 解決の基準にする repository root。省略時は cwd

依存が未導入なら最初に `npm install` が必要です。

## workspace_fs での使い方

`workspace_fs_server` から default plugin として実行する想定です。通常は出力先に `{OUTPUT_DIRECTORY}` を渡します。

```toml
[[plugin]]
name = "md-preview"
runner = "default"
allow = ["alice_browser"]
mount = "/md/"

[plugin.md_preview]
macro_path = "./macros.txt"

[plugin.md_preview.md_viewer]
additional_js = ["assets/header.js"]
```

- `mount` 先に生成された `md_preview.html` や `directory_view.html` を公開する
- plugin 設定は `WORKSPACE_FS_PLUGIN_SETTINGS_JSON` 経由で `build.mjs` に渡る
- viewer ごとの head asset 注入は `[plugin.md_preview.<viewer>]` で指定する

## 生成物

主に次のファイルを出力します。

- `md_preview.html`: markdown viewer
- `md_editor.html`: markdown editor
- `directory_view.html`: directory viewer
- `markdown_viewer.js`: browser 側の markdown renderer
- `transform_runner.js`: markdown 変換 hook の実行器
- `katex_transform.js`: 既定の KaTeX transform
- `link_index.json`: repository 内の markdown から作る wiki link index
- `macros.txt`: `macro_path` を指定した場合のみ生成

KaTeX の CSS と font は `vendor/katex/` にコピーします。

## markdown 表示

- markdown は browser 側で render する
- 既定で KaTeX transform を入れる
- `> [!NOTE]` や `> [!TIP]` のような GitHub-style alert を専用 block に変換する
- `[[plugin.md_preview.transform]]` を指定すると、既定の KaTeX transform の後ろに追加で実行する

transform の各 entry では次を指定します。

- `name`
- `url`
- `entrypoint`
- その他の field は `options` としてそのまま渡る

## macros

`[plugin.md_preview].macro_path` を指定すると、repository 内の file を `macros.txt` として出力先へコピーします。

- `macro_path` は repository root 相対
- file が存在しない場合は build error
- viewer は `macros.txt` を暗黙には読み込まない

## viewer ごとの追加 asset

次の table を使えます。

- `[plugin.md_preview.md_viewer]`
- `[plugin.md_preview.md_editor]`
- `[plugin.md_preview.directory_view]`

各 table では次の配列を指定できます。

- `additional_js`
- `additional_module_js`
- `additional_css`

これらの path は repository 相対で、生成される HTML には `/assets/header.js` のような root-relative URL として埋め込みます。絶対 path や repository root の外に出る path は拒否します。

## directory view

`directory_view.html` は `workspace_fs` の一覧 API を使って repository を browse します。

- `GET /PATH/` の一覧結果を使う
- 各 entry の path info が読めない場合、その entry は表示しない
- ignore された path は一覧 API 側で消えるので表示されない
- policy を bypass せず、読めない path にはアクセスしない

directory card は子 directory へリンクし、`README.md` があれば preview 文の候補として使います。file card は現在 `md`、`txt`、`rs` だけを表示対象にし、markdown file は `md_preview.html` へ、それ以外は raw path へリンクします。

## link index

`link_index.json` は repository 内の `.md` file を走査して生成します。

- 走査対象は repository root 以下
- `.repo/`、`.git/`、`node_modules/`、`target/` は除外
- wiki link の term ごとに参照元 page 一覧を持つ

directory viewer や markdown viewer はこの index を使って backlink 風の link page を表示します。
