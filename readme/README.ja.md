[中文](../README.md) | [English](README.en.md) | [日本語](README.ja.md)

# Erika

Rust で実装された独立型メディアプレーヤーエンジンです。

再生制御、タイミング同期、音声出力、字幕・弾幕合成、そしてネイティブレンダリングは Erika が担います。Flutter や Swift など C FFI に対応したホスト環境は、レンダリングサーフェスの提供とコマンドの送信のみを行い、フレームデータには一切触れません。

## 機能

- ローカルファイルおよび HTTP Range メディアソース
- ハードウェアアクセラレーションによる動画デコード（macOS では VideoToolbox を使用）
- HDR/EDR を見据えた Metal と CAMetalLayer によるネイティブ表示パス
- 字幕対応：SRT、WebVTT、ASS
- 弾幕対応：Bilibili XML および JSON-lines 形式、衝突回避レーン配置付き
- CoreAudio による音声出力
- 不透明な C ABI — Swift、Dart FFI、C/C++、または任意の Rust クレートから呼び出し可能

wgpu を通じたクロスプラットフォーム対応は計画中です。現在のターゲットは macOS 14+ です。

## 組み込み

Erika は二種類の C ABI を提供します：

- **`ErikaHandle`** — 再生制御とイベントポーリング。ホスト側で独自のレンダリングループを持つ場合、または再生制御のみが必要な場合に使用します。
- **`ErikaPresenterHandle`** — プレゼンタースタック全体（プレーヤー、レンダラー、音声、オーバーレイ）を Erika が管理します。ホストはネイティブサーフェスを提供し、ディスプレイタイマーから `render_tick` を呼び出すだけで済みます。

Flutter との統合方法および macOS HDR 組み込み戦略については [`docs/flutter_embedding.md`](../docs/flutter_embedding.md) を参照してください。

## 現在の状態

初期段階のエンジン基盤です。macOS の再生パスはエンドツーエンドで動作しています。プロダクション利用にはまだ対応していません。

## ライセンス

Rust ワークスペースは MPL-2.0 です。ネイティブ依存関係は `xtask` で別途管理されており、デフォルトの FFmpeg ビルドは LGPL 準拠で、GPL コンポーネントはオプトイン方式です。
