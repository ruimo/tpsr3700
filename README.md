# tpsr3700

Linux and macOS driver for the **KING JIM TEPRA PRO SR3700P** label printer.

Print text labels directly from the command line — no Windows, no vendor software required.

> **Japanese / 日本語版は下部にあります。** → [日本語 README](#日本語)

> **Disclaimer:** This project is independent and is **not affiliated with, endorsed by, or supported by KING JIM Co., Ltd.** Please do **not** contact KING JIM regarding this software.

---

## Features

- Print arbitrary text on 6 mm, 12 mm, and 18 mm tapes
- Auto-detect the loaded tape width by querying the printer
- Auto-select the largest font size that fits the tape
- Multi-line printing via explicit `\n` line breaks — works with or without `--max-tape-length-mm`
- Wrap-to-length: wrap text to a fixed label length, with a configurable maximum line count derived from the tape width (minimum line height: 3 mm)
- Left / center / right alignment per line (with `--max-tape-length-mm`)
- Dump the raw job bytes to a file (useful for debugging without hardware)

## Requirements

- Rust toolchain (edition 2024)
- `libusb`:
  - Debian/Ubuntu: `sudo apt-get install libusb-1.0-0-dev`
  - macOS: `brew install libusb`

No font installation is required — **BIZ UDPゴシック Regular** is bundled
directly in the binary. To use a different font, pass `--font <PATH>` with
any `.ttf`/`.otf`/`.ttc` file.

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
| `--left-margin-mm <MM>` | `10.0` | Blank feed margin (mm) added before the printed content (left side as the label exits the printer). |
| `--right-margin-mm <MM>` | `3.0` | Blank feed margin (mm) added after the printed content (right side). |
| `--top-margin <DOTS>` | `2` | Non-printing margin at the top edge of the tape, in dots. |
| `--bottom-margin <DOTS>` | `8` | Non-printing margin at the bottom edge of the tape, in dots. |
| `--max-tape-length-mm <MM>` | — | Total label length in mm (including margins). Enables multi-line wrapping when the text does not fit in one line. |
| `--align <ALIGN>` | `left` | Per-line alignment: `left`, `center`, or `right`. Only meaningful with `--max-tape-length-mm`. |
| `--font <PATH>` | bundled BIZ UDPゴシック | Path to a `.ttf`/`.otf`/`.ttc` font file. Overrides the bundled font. |
| `--dump <PATH>` | — | Write the raw job bytes to a file (in addition to, or instead of, printing). |
| `--no-print` | — | Skip sending the job to the printer. Useful with `--dump`. |

### Examples

```sh
# Print "Hello" on the loaded tape (width auto-detected)
tpsr3700 "Hello"

# Print on 12 mm tape, explicit font size
tpsr3700 --tape-mm 12 --size 24 "Hello"

# Two lines with \n — no --max-tape-length-mm needed; font size is
# automatically halved so each line fits its band
tpsr3700 --tape-mm 18 "Line1\nLine2"

# Multi-line: wrap to a 50 mm label, center-aligned
tpsr3700 --tape-mm 18 --max-tape-length-mm 50 --align center "Hello World"

# Explicit line break with \n inside a fixed-length label
tpsr3700 --tape-mm 18 --max-tape-length-mm 40 "Line1\nLine2"

# Dump job bytes without printing (useful for debugging)
tpsr3700 --tape-mm 12 --no-print --dump job.prn "Test"
```

### Multi-line rules

- The minimum height of one printed line is **3 mm**.
  A 6 mm tape fits at most **2 lines**; 12 mm fits 4; 18 mm fits 6.
- **`\n` in the text always forces a line break**, regardless of whether `--max-tape-length-mm` is set.
  Each line is rendered into an equal vertical band.
- When `--size` is omitted and the text contains `\n`, the font size is automatically divided by the number of lines so each band gets the largest size that fits.
- When `--max-tape-length-mm` is given and the text overflows one line, the tool automatically increases the line count (up to the tape's maximum) until everything fits, wrapping character by character.
- `--align` applies to each line independently (only meaningful with `--max-tape-length-mm`).

## USB permissions (Linux)

The tool communicates with the printer via libusb. On most Linux systems, regular users do not have **read/write** access to USB device nodes by default. Without it you will get a "permission denied" error when running the tool.

> **macOS:** IOKit grants userspace access to USB devices automatically — no extra configuration is needed.

The recommended fix on Linux is a persistent udev rule that grants read/write access to members of the `lp` group. Create the file:

```
# /etc/udev/rules.d/99-tepra.rules
SUBSYSTEM=="usb", ATTRS{idVendor}=="0d8a", ATTRS{idProduct}=="0103", GROUP="lp", MODE="0660"
```

`GROUP="lp"` restricts access to the `lp` group and `MODE="0660"` grants read/write to that group only (safer than opening it to all users).

Or create it in one command:

```sh
echo 'SUBSYSTEM=="usb", ATTRS{idVendor}=="0d8a", ATTRS{idProduct}=="0103", GROUP="lp", MODE="0660"' \
    | sudo tee /etc/udev/rules.d/99-tepra.rules
```

Then add your user to the `lp` group, reload the rules, and re-trigger the device:

```sh
sudo usermod -aG lp $USER
sudo udevadm control --reload && sudo udevadm trigger
```

After that, unplug and re-plug the printer and start a new login session (or run `newgrp lp` in the current shell). The rule persists across reboots.

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
captures/           Raw USB capture of the tape-width probe (.pcap)
docs/protocol.md    Protocol documentation
src/font/
  BIZUDPGothic-Regular.ttf   BIZ UDPゴシック Regular, bundled in the binary
  OFL.txt                    Font license (SIL Open Font License 1.1)
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

Linux・macOS 用 **KING JIM TEPRA PRO SR3700P** ラベルプリンタードライバーです。

Windows も純正ソフトも不要で、コマンドラインから直接テキストラベルを印刷できます。

> **免責事項:** 本プログラムは KING JIM 株式会社とは無関係です。本ソフトウェアについて KING JIM 社へお問い合わせをしないでください。

---

## 機能

- 6 mm・12 mm・18 mm テープへの任意テキスト印刷
- プリンターへの問い合わせによるテープ幅自動検出
- テープ幅に収まる最大フォントサイズの自動選択
- **複数行印刷**: テキスト中の `\n` で改行 — `--max-tape-length-mm` の有無に関わらず使用可能
- ラベル長指定による折り返し（テープ幅から最大行数を自動計算、最低行高さ 3 mm）
- 行ごとの左寄せ / 中央揃え / 右寄せ（`--max-tape-length-mm` 使用時）
- 生のジョブバイト列をファイルにダンプ（ハードウェアなしでのデバッグに便利）

## 必要なもの

- Rust ツールチェーン（edition 2024）
- `libusb`:
  - Debian/Ubuntu: `sudo apt-get install libusb-1.0-0-dev`
  - macOS: `brew install libusb`

フォントのインストールは不要です。**BIZ UDPゴシック Regular** をバイナリに直接埋め込んでいます。
別のフォントを使いたい場合は `--font <PATH>` で `.ttf`/`.otf`/`.ttc` ファイルを指定してください。

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
| `--left-margin-mm <MM>` | `10.0` | 印字内容の前（ラベルがプリンターから出る側＝左端）に追加する余白（mm）。 |
| `--right-margin-mm <MM>` | `3.0` | 印字内容の後（右端）に追加する余白（mm）。 |
| `--top-margin <DOTS>` | `2` | テープ上端の非印字マージン（ドット数）。 |
| `--bottom-margin <DOTS>` | `8` | テープ下端の非印字マージン（ドット数）。 |
| `--max-tape-length-mm <MM>` | — | ラベル全長（mm、マージン込み）。指定すると、1 行に収まらないテキストを複数行に折り返す。 |
| `--align <ALIGN>` | `left` | 各行の横方向揃え: `left`（左寄せ）・`center`（中央）・`right`（右寄せ）。`--max-tape-length-mm` 指定時のみ有効。 |
| `--font <PATH>` | バンドル済み BIZ UDPゴシック | フォントファイルのパス（`.ttf`/`.otf`/`.ttc`）。バンドルフォントを上書きします。 |
| `--dump <PATH>` | — | 生のジョブバイト列をファイルに書き出す（印刷と併用可）。 |
| `--no-print` | — | プリンターへの送信をスキップ。`--dump` と組み合わせて使用。 |

### 使用例

```sh
# テープ幅を自動検出して "Hello" を印刷
tpsr3700 "Hello"

# 12 mm テープ、フォントサイズ 24pt を指定して印刷
tpsr3700 --tape-mm 12 --size 24 "Hello"

# \n で 2 行印刷 — --max-tape-length-mm 不要
# --size 省略時はフォントサイズを行数で自動分割
tpsr3700 --tape-mm 18 "1行目\n2行目"

# 50 mm のラベルに複数行・中央揃えで折り返し印刷
tpsr3700 --tape-mm 18 --max-tape-length-mm 50 --align center "Hello World"

# ラベル長を指定して \n で明示的に改行
tpsr3700 --tape-mm 18 --max-tape-length-mm 40 "1行目\n2行目"

# 印刷せずにジョブバイト列をファイルにダンプ（デバッグ用）
tpsr3700 --tape-mm 12 --no-print --dump job.prn "Test"
```

### 複数行印刷のルール

- 1 行の最低高さは **3 mm** です。
  6 mm テープは最大 **2 行**、12 mm は 4 行、18 mm は 6 行まで印字できます。
- **テキスト中の `\n` は常に改行**として扱われます（`--max-tape-length-mm` の有無を問いません）。
  各行はテープの縦方向を等分したバンドに描画されます。
- `--size` を省略した場合、`\n` の行数に応じてフォントサイズを自動的に分割し、各バンドに収まる最大サイズを使用します。
- `--max-tape-length-mm` を指定した場合、1 行に収まらないテキストは自動的に行数を増やして（最大行数まで）文字単位で折り返します。
- `--align` は各行に個別に適用されます（`--max-tape-length-mm` 使用時のみ有効）。

## USB アクセス権の設定（Linux）

このツールは libusb 経由でプリンターと通信します。多くの Linux 環境では、一般ユーザーはデフォルトで USB デバイスノードへの**読み書きアクセス権**を持っていません。権限がない場合、ツール実行時に "permission denied" エラーが発生します。

> **macOS:** IOKit がユーザー空間への USB アクセスを自動で許可するため、追加設定は不要です。

推奨される対処法は Linux での udev ルール設定です。`lp` グループのメンバーに読み書きアクセスを付与する永続的なルールを作成してください：

```
# /etc/udev/rules.d/99-tepra.rules
SUBSYSTEM=="usb", ATTRS{idVendor}=="0d8a", ATTRS{idProduct}=="0103", GROUP="lp", MODE="0660"
```

`GROUP="lp"` でアクセスを `lp` グループに限定し、`MODE="0660"` でそのグループにのみ読み書き権限を付与します（全ユーザーに開放するより安全です）。

次のコマンド 1 行で作成することもできます：

```sh
echo 'SUBSYSTEM=="usb", ATTRS{idVendor}=="0d8a", ATTRS{idProduct}=="0103", GROUP="lp", MODE="0660"' \
    | sudo tee /etc/udev/rules.d/99-tepra.rules
```

続けて、自分のユーザーを `lp` グループに追加し、ルールを反映してデバイスを再トリガーします：

```sh
sudo usermod -aG lp $USER
sudo udevadm control --reload && sudo udevadm trigger
```

プリンターを抜き差しし、新しいログインセッションを開始してください（または現在のシェルで `newgrp lp` を実行）。ルールは再起動後も有効です。

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
captures/           テープ幅検出時の USB キャプチャ（.pcap）
docs/protocol.md    プロトコル仕様書
src/font/
  BIZUDPGothic-Regular.ttf   バイナリに埋め込まれる BIZ UDPゴシック Regular
  OFL.txt                    フォントのライセンス（SIL Open Font License 1.1）
tests/fixtures/     バイト完全一致テスト用ゴールデン .prn ファイル
```

## テストの実行

```sh
cargo test
```

## ライセンス

[Apache License, Version 2.0](LICENSE) の下で提供されます。
