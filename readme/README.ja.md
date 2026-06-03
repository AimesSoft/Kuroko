[中文](../README.md) | [English](README.en.md) | [日本語](README.ja.md)

# Kuroko

Rust で実装された独立型メディアプレーヤーエンジンです。

再生制御、タイミング同期、音声出力、字幕・弾幕合成、そしてネイティブレンダリングは Kuroko が担います。Flutter や Swift など C FFI に対応したホスト環境は、レンダリングサーフェスの提供とコマンドの送信のみを行い、フレームデータには一切触れません。

## 機能

- ローカルファイルおよび HTTP Range メディアソース
- ハードウェアアクセラレーションによる動画デコード（macOS では VideoToolbox を使用）
- HDR/EDR を見据えた Metal と CAMetalLayer によるネイティブ表示パス
- 字幕対応：SRT、WebVTT、ASS
- 弾幕対応：Bilibili XML および JSON-lines 形式、衝突回避レーン配置付き
- CoreAudio による音声出力
- 不透明な C ABI — Swift、Dart FFI、C/C++、または任意の Rust クレートから呼び出し可能

クロスプラットフォーム対応のための wgpu レンダリングバックエンドを開発中です。現在は macOS 14+ を中心としています。

## ビルド

ネイティブ依存関係、Nix 環境、サンプルの実行コマンドは [`docs/development.md`](../docs/development.md) にまとめています。

## 組み込み

Kuroko は二種類の C ABI を提供します：

- **`KurokoHandle`** — 再生制御とイベントポーリング。ホスト側で独自のレンダリングループを持つ場合、または再生制御のみが必要な場合に使用します。
- **`KurokoPresenterHandle`** — プレゼンタースタック全体（プレーヤー、レンダラー、音声、オーバーレイ）を Kuroko が管理します。ホストはネイティブサーフェスを提供し、ディスプレイタイマーから `render_tick` を呼び出すだけで済みます。

Flutter との統合方法および macOS HDR 組み込み戦略については [`docs/flutter_embedding.md`](../docs/flutter_embedding.md) を参照してください。

## リポジトリ構成

```text
crates/kuroko              プレーヤーエンジン
crates/kuroko_capi         不透明ハンドルの C ABI
crates/kuroko_ffmpeg_sys   FFmpeg 低レベルバインディング
docs/                      アーキテクチャ・組み込み・開発ノート
examples/                  サブシステムごとの実行可能なサンプル
xtask/                     ネイティブ依存関係のオーケストレーション
```

## 現在の状態

初期段階のエンジン基盤です。macOS の再生パスはエンドツーエンドで動作しています。プロダクション利用にはまだ対応していません。

## ライセンス

Rust ワークスペースは MPL-2.0 です。ネイティブ依存関係は `xtask` で別途管理されており、デフォルトの FFmpeg ビルドは LGPL 準拠で、GPL コンポーネントはオプトイン方式です。
