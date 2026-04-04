# ほしい機能
- static file としてのビルドは可能か？
  現状だと、 policy で GET が許されているファイルを全部と、 plugin 以下の mount をそのファイル名に直せば、
  static file にしてまとめることができるように思える。
  API 経由で plugin の起動ができない点にだけ注意。

# internal な todo
- `workspace.rs` が大きすぎるので分ける: fileservice/policyservice/infoservice/pluginservice/
- エラーをちゃんと分けて `error.rs` にまとめる。
- `config.toml` の validation を複数の method に分ける。
