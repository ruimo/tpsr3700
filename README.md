# tpsr3700

Linux driver for the **KING JIM TEPRA PRO SR3700P** label printer.

Print text labels directly from the command line — no Windows, no vendor software required.

> **Japanese / 日本語版は下部にあります。** → [日本語 README](#日本語)

> **Disclaimer:** This project is independent and is **not affiliated with, endorsed by, or supported by KING JIM Co., Ltd.** Please do **not** contact KING JIM regarding this software.

---

## Features

- Print arbitrary text on 6 mm, 12 mm, and 18 mm tapes
- Auto-detect the loaded tape width by querying the printer
- Auto-select the largest font size that fits the tape
- Multi-line printing: wrap text to a fixed label length, with a configurable maximum line count derived from the tape width (minimum line height: 3 mm)
- Explicit line breaks with `\n` in the text
- Left / center / right alignment per line
- Dump the raw job bytes to a file (useful for debugging without hardware)

## Requirements

- Rust toolchain (edition 2024)
- `libusb` development headers (`libusb-1.0-dev` on Debian/Ubuntu)
- A TrueType/OpenType font — defaults to `/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc`

## Build

```sh
cargo build --release
```

## Usage

```
tpsr3700 [OPTIONS] <TEXT>
```

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--tape-mm <MM>` | auto-detect | Tape width in mm. Supported: `6`, `12`, `18`. Omit to query the printer. |
| `--size <PT>` | auto | Font size in points (as shown in the TEPRA app, e.g. `36`). Omit for the largest size that fits. |
| `--margin-mm <MM>` | `3.0` | Blank feed margin added before and after the printed content. |
| `--tape-length-mm <MM>` | — | Total label length in mm (including margins). Enables multi-line wrapping when the text does not fit in one line. |
| `--align <ALIGN>` | `left` | Per-line alignment: `left`, `center`, or `right`. Only meaningful with `--tape-length-mm`. |
| `--font <PATH>` | Noto Sans CJK JP | Path to a `.ttf`/`.otf`/`.ttc` font file. |
| `--dump <PATH>` | — | Write the raw job bytes to a file (in addition to, or instead of, printing). |
| `--no-print` | — | Skip sending the job to the printer. Useful with `--dump`. |

### Examples

```sh
# Print "Hello" on the loaded tape (width auto-detected)
tpsr3700 "Hello"

# Print on 12 mm tape, explicit font size
tpsr3700 --tape-mm 12 --size 24 "Hello"

# Multi-line: wrap to a 50 mm label, center-aligned
tpsr3700 --tape-mm 18 --tape-length-mm 50 --align center "Hello World"

# Explicit line break with \n
tpsr3700 --tape-mm 18 --tape-length-mm 40 $'Line1\nLine2'

# Dump job bytes without printing (useful for debugging)
tpsr3700 --tape-mm 12 --no-print --dump job.prn "Test"
```

### Multi-line rules

- The minimum height of one printed line is **3 mm**.  
  A 6 mm tape fits at most **2 lines**; 12 mm fits 4; 18 mm fits 6.
- When `--tape-length-mm` is given and the text overflows one line, the tool automatically increases the line count (up to the tape's maximum) until everything fits, wrapping character by character.
- `\n` in the text always forces a line break, regardless of width.
- `--align` applies to each line independently.

## USB permissions

The tool communicates with the printer via libusb. On most Linux systems you need either root, or a udev rule granting your user access:

```
# /etc/udev/rules.d/99-tepra.rules
SUBSYSTEM=="usb", ATTRS{idVendor}=="0d8a", ATTRS{idProduct}=="0103", MODE="0666"
```

Then reload rules:

```sh
sudo udevadm control --reload && sudo udevadm trigger
```

## Project layout

```
src/
  main.rs           CLI entry point (clap argument parsing)
  font/mod.rs       Font rendering: text → raster rows (fontdue)
  tepra/
    mod.rs          TapeWidth enum and tape geometry constants
    builder.rs      Assembles the full print job byte stream
    frame.rs        ESC { } command frame encoder
    header.rs       Fixed 2179-byte job header builder
    raster.rs       Single raster row encoder
  usb/              libusb/rusb transport (bulk write + tape detection)
assets/
  header_18mm_template.bin   Base template for the job header
captures/           Raw .prn job files captured for various configurations
docs/protocol.md    Protocol documentation
tests/fixtures/     Golden .prn files used in byte-exact reconstruction tests
```

## Running the tests

```sh
cargo test
```

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

---

<a name="日本語"></a>

# tpsr3700（日本語）

Linux 用 **KING JIM TEPRA PRO SR3700P** ラベルプリンタードライバーです。

Windows も純正ソフトも不要で、Linux のコマンドラインから直接テキストラベルを印刷できます。

> **免責事項:** 本プログラムは KING JIM 株式会社とは無関係です。本ソフトウェアについて KING JIM 社へお問い合わせをしないでください。

---

## 機能

- 6 mm・12 mm・18 mm テープへの任意テキスト印刷
- プリンターへの問い合わせによるテープ幅自動検出
- テープ幅に収まる最大フォントサイズの自動選択
- **複数行印刷**: ラベル長を指定して折り返し（テープ幅から最大行数を自動計算、最低行高さ 3 mm）
- テキスト中の `\n` による明示的な改行
- 行ごとの左寄せ / 中央揃え / 右寄せ
- 生のジョブバイト列をファイルにダンプ（ハードウェアなしでのデバッグに便利）

## 必要なもの

- Rust ツールチェーン（edition 2024）
- `libusb` 開発ヘッダー（Debian/Ubuntu では `libusb-1.0-dev`）
- TrueType/OpenType フォント（デフォルト: `/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc`）

## ビルド

```sh
cargo build --release
```

## 使い方

```
tpsr3700 [オプション] <テキスト>
```

### オプション一覧

| オプション | デフォルト | 説明 |
|-----------|-----------|------|
| `--tape-mm <MM>` | 自動検出 | テープ幅（mm）。`6`・`12`・`18` をサポート。省略時はプリンターに問い合わせ。 |
| `--size <PT>` | 自動 | フォントサイズ（ポイント、TEPRA アプリの表示と同じ。例: `36`）。省略時はテープに収まる最大サイズ。 |
| `--margin-mm <MM>` | `3.0` | 印字内容の前後に追加する余白（mm）。 |
| `--tape-length-mm <MM>` | — | ラベル全長（mm、マージン込み）。指定すると、1 行に収まらないテキストを複数行に折り返す。 |
| `--align <ALIGN>` | `left` | 各行の横方向揃え: `left`（左寄せ）・`center`（中央）・`right`（右寄せ）。`--tape-length-mm` 指定時のみ有効。 |
| `--font <PATH>` | Noto Sans CJK JP | フォントファイルのパス（`.ttf`/`.otf`/`.ttc`）。 |
| `--dump <PATH>` | — | 生のジョブバイト列をファイルに書き出す（印刷と併用可）。 |
| `--no-print` | — | プリンターへの送信をスキップ。`--dump` と組み合わせて使用。 |

### 使用例

```sh
# テープ幅を自動検出して "Hello" を印刷
tpsr3700 "Hello"

# 12 mm テープ、フォントサイズ 24pt を指定して印刷
tpsr3700 --tape-mm 12 --size 24 "Hello"

# 50 mm のラベルに複数行・中央揃えで折り返し印刷
tpsr3700 --tape-mm 18 --tape-length-mm 50 --align center "Hello World"

# \n で明示的に改行
tpsr3700 --tape-mm 18 --tape-length-mm 40 $'1行目\n2行目'

# 印刷せずにジョブバイト列をファイルにダンプ（デバッグ用）
tpsr3700 --tape-mm 12 --no-print --dump job.prn "Test"
```

### 複数行印刷のルール

- 1 行の最低高さは **3 mm** です。  
  6 mm テープは最大 **2 行**、12 mm は 4 行、18 mm は 6 行まで印字できます。
- `--tape-length-mm` を指定した場合、1 行に収まらないテキストは自動的に行数を増やして（最大行数まで）文字単位で折り返します。
- テキスト中の `\n` は幅に関係なく常に改行として扱われます。
- `--align` は各行に個別に適用されます。

## USB アクセス権の設定

このツールは libusb 経由でプリンターと通信します。多くの Linux 環境では root 権限か、以下の udev ルールが必要です。

```
# /etc/udev/rules.d/99-tepra.rules
SUBSYSTEM=="usb", ATTRS{idVendor}=="0d8a", ATTRS{idProduct}=="0103", MODE="0666"
```

ルールを反映するには:

```sh
sudo udevadm control --reload && sudo udevadm trigger
```

## プロジェクト構成

```
src/
  main.rs           CLI エントリーポイント（clap 引数解析）
  font/mod.rs       フォントレンダリング: テキスト → ラスタ行（fontdue 使用）
  tepra/
    mod.rs          TapeWidth 列挙型とテープのジオメトリ定数
    builder.rs      印刷ジョブのバイト列を組み立て
    frame.rs        ESC { } コマンドフレームエンコーダー
    header.rs       固定長 2179 バイトのジョブヘッダービルダー
    raster.rs       ラスタ行エンコーダー
  usb/              libusb/rusb トランスポート（バルク書き込みとテープ幅検出）
assets/
  header_18mm_template.bin   ジョブヘッダーのベーステンプレート
captures/           各種設定でキャプチャした生 .prn ファイル
docs/protocol.md    プロトコル仕様書
tests/fixtures/     バイト完全一致テスト用ゴールデン .prn ファイル
```

## テストの実行

```sh
cargo test
```

## ライセンス

[Apache License, Version 2.0](LICENSE) の下で提供されます。
