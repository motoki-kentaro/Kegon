# Issue #66 Phase 0B: Shell Reported Status 通信方式の比較 PoC 報告書

> [!NOTE]
> 本書は Issue #66 の検討過程における通信方式比較 PoC 報告書（設計履歴）です。確定した最終仕様は `docs/architecture/terminal.md` を参照してください。本書で比較検討された案B・C・D等は不採用となった経緯を含みます。

- 状態: Completed / Proposed
- 対象 Issue: https://github.com/motoki-kentaro/Kegon/issues/66
- 作業場所: `C:\works\kegon-66-shell-status`
- ブランチ: `feature/66-powershell-reported-status`
- 基点: `origin/main` (`aa354c9963ab4ac610e3a33f5e185f9dda524a9b`)
- 日時: 2026-10-18

---

## 1. 概要と検証目的

Phase 0A において、PowerShell の prompt 入口で `$?`（PowerShell 報告成否）を既存ユーザー環境へ副作用を与えずに捕捉できること（33/33 テスト PASS、100% の状態引継ぎ一致、`$LASTEXITCODE` / `$Error` / `$ErrorActionPreference` の完全非干渉）を実証しました。

本 Phase 0B では、捕捉した PowerShell 報告成否（`$?`）を OSC 133 による実行境界と正確に対応付けて Kegon に伝送する**通信方式（プロトコル構文、相関モデル、異常時縮退、相互運用性）**を比較検討し、隔離環境における実測 PoC を完了しました。

本検証はプロトコル設計およびテストハーネス（`assets/shell-integration/tests/`）のみで実施し、**製品コード（`src/`、本番 `powershell.ps1`）には一切変更を加えていません。**

---

## 2. 比較対象方式

以下の 4 方式を設計・実装し、隔離 PoC 環境で直接比較しました。

| 方式 | 概要 | ワイヤ構文例 |
|---|---|---|
| **案A: OSC 133 D 引数拡張** | OSC 133 D の終了コードスロット直後に独自 key-value 引数を配置 | `OSC 133;D;[code];kegon.v=1;kegon.sid=<id>;kegon.seq=<n>;kegon.ps=<status> BEL/ST`<br>数値なし: `OSC 133;D;;kegon.v=1;...` |
| **案B: 独立通知 (OSC 1337 SetUserVar)** | iTerm2/WezTerm 由来のユーザー変数 OSC 1337 でメタデータを独立送信し、従来の 133;C/D と併用 | `OSC 1337;SetUserVar=kegon.meta=<base64(...)> BEL/ST`<br>直後に `OSC 133;D BEL/ST` |
| **案C (代替案1): OSC 133 P Property** | FinalTerm/WezTerm 仕様の Property 通知 `OSC 133;P` でメタデータを送信し、直後に 133;D | `OSC 133;P;kegon.v=1;kegon.sid=<id>;kegon.seq=<n>;kegon.ps=<status> BEL/ST`<br>直後に `OSC 133;D BEL/ST` |
| **案D (代替案2 / 案A改良): 構造化 D 判別子** | 案A の空スロット `;;` を廃止し、D 直後のトークンが数値か key-value かを自己判別 | `OSC 133;D;kegon.ps=<status>;kegon.v=1;kegon.sid=<id>;kegon.seq=<n> BEL/ST`<br>数値あり: `OSC 133;D;42;kegon.ps=...` |

---

## 3. 必須検証 10 項目の実測結果

本番コードを変更せず、独立テストフィクスチャ `phase0b_protocol_poc.rs`（`rustc` ビルド）およびクロスシェルランナー `phase0b-protocol-poc.ps1` を用いて検証を実施しました。

### 3.1. Rust 単体検証結果 (Items 1〜8)

| 項目 | 検証内容 | 結果 | 備考 |
|---|---|:---:|---|
| **1. 成否往復** | `ReportedSuccess`, `ReportedFailure`, `Unknown` のエンコード・送受信・復元 | **PASS** | 3 状態すべて非可逆変換なく完全一致 |
| **2. C/D 相関** | `(session_id, seq)` による同一コマンドへの対応付け | **PASS** | 一致時正常受理、seq 不一致・sid 不一致は即座に `Desynchronized` / `Unknown` |
| **3. 数値 D との直交性** | `osc133_exit` と `shell_status` の完全独立管理 | **PASS** | `(Some(0), ReportedFailure)`、`(Some(42), ReportedSuccess)`、`(None, ReportedSuccess)` が相互に汚染・推測されない |
| **4. チャンク分割・終端** | 1 バイト分割、任意分割、BEL (`\x07`) / ST (`\x1b\\`) 終端 | **PASS** | 全分割位置で正常復元。ST (`ESC \`) も BEL と同等に処理 |
| **5. 欠落・重複・逆順** | D 欠落、メタデータ重複、順序逆転の縮退 | **PASS** | D 欠落時は次プロンプトで `MissingD`。重複・逆順は `Desynchronized` / `Unknown` へ安全縮退 |
| **6. 複数タブ分離** | Tab 1 / Tab 2 の並行・交互トラフィック | **PASS** | TabId + session/seq による完全分離。状態混入 0 件 |
| **7. 異常系・上限** | 未知バージョン (`v=99`)、不正文法、8KB 超過ペイロード | **PASS** | 不正値は `Unknown` 縮退。8KB 超過は `OscScanner` がメモリ消費なく破棄 |
| **8. 旧 Kegon 互換性** | 現行 Kegon パーサー (`src/terminal/osc.rs`) への入力 | **PASS** | 案A/D は `CommandFinished(None)`、案B/C は `None`（無視）。パニックや誤動作なし |

### 3.2. 実シェルプロセス E2E パイプ検証 (Item 9: PowerShell 7 & 5.1)

実シェルプロセス（`pwsh` 7.6.7 および `powershell.exe` 5.1.26100.9587）から生成された標準出力バイトストリームを、Rust パーサープロセスへ標準入力経由で直接パイプして検証しました。

| 実シェル | 方式 | 終了コードなし成否 | 終了コード 42 + 失敗 | 終了コード 0 + 成否 | 終了コード 0 + Cmdlet 失敗 | 判定 |
|---|---|:---:|:---:|:---:|:---:|:---:|
| **PowerShell 7 (`pwsh`)** | 案A (D 引数) | OK | OK | OK | OK | **PASS** |
| PowerShell 7 (`pwsh`) | 案B (1337 SetUserVar) | OK | OK | OK | OK | **PASS** |
| PowerShell 7 (`pwsh`) | 案C (133 P Property) | OK | OK | OK | OK | **PASS** |
| PowerShell 7 (`pwsh`) | 案D (構造化 D 判別子) | OK | OK | OK | OK | **PASS** |
| PowerShell 7 (`pwsh`) | 旧 Kegon 互換性 | OK (`CommandFinished(None)`) | OK | OK | OK | **PASS** |
| **Windows PowerShell 5.1** | 案A (D 引数) | OK | OK | OK | OK | **PASS** |
| Windows PowerShell 5.1 | 案B (1337 SetUserVar) | OK | OK | OK | OK | **PASS** |
| Windows PowerShell 5.1 | 案C (133 P Property) | OK | OK | OK | OK | **PASS** |
| Windows PowerShell 5.1 | 案D (構造化 D 判別子) | OK | OK | OK | OK | **PASS** |
| Windows PowerShell 5.1 | 旧 Kegon 互換性 | OK (`CommandFinished(None)`) | OK | OK | OK | **PASS** |

### 3.3. PowerShell プロンプト内送信性能ベンチマーク (1,000 回連続実行)

各方式におけるプロンプト呼び出し当たりのオーバーヘッドを実測しました。

| 実機シェル | 方式 | 1,000 回合計時間 | 1 コマンド当たり所要時間 | 評価 |
|---|---|:---:|:---:|---|
| **PowerShell 7** | 案A (D 引数) | 32 ms | **32 μs** | 極めて軽量（文字列補間 1 回） |
| PowerShell 7 | 案B (SetUserVar) | 74 ms | **74 μs** | 案A 比 2.3 倍（UTF-8 バイト変換 + Base64 生成） |
| PowerShell 7 | 案C (133 P) | 34 ms | **34 μs** | 軽量（OSC 出力 2 回） |
| PowerShell 7 | 案D (構造化 D) | 34 ms | **34 μs** | 極めて軽量（文字列補間 1 回） |
| **Windows PowerShell 5.1** | 案A (D 引数) | 13 ms | **13 μs** | 極めて軽量 |
| Windows PowerShell 5.1 | 案B (SetUserVar) | 27 ms | **27 μs** | 案A 比 2.1 倍 |
| Windows PowerShell 5.1 | 案C (133 P) | 15 ms | **15 μs** | 軽量 |
| Windows PowerShell 5.1 | 案D (構造化 D) | 14 ms | **14 μs** | 極めて軽量 |

### 3.4. 非 OSC 環境・外部端末での安全性 (Item 10)

- **非 ANSI / ダンプ環境（プレーンテキストログ等）**:
  - 全方式ともに `ESC ] ... BEL` 形式のエスケープシーケンスであるため、非 ANSI 環境ではエスケープ文字（`^[...`）として出力される点は同一。
- **外部端末エミュレータ（iTerm2, WezTerm, VS Code, Windows Terminal, Alacritty）**:
  - **案A の課題（実機・資料分析）**:
    - `133;D;;kegon...` のように空スロット `;;` が存在する場合、C 言語系パーサーで第 2 パラメータを `atoi(param[1])` で読み取ると `atoi("") == 0` となり、**「数値終了コードなし」が「終了コード 0 (成功)」と誤認されるリスク**がある（VS Code は `parseInt` で `NaN` になり問題ないが、他端末で潜在的リスク）。
  - **案B の特性**:
    - `OSC 1337 SetUserVar` は iTerm2 / WezTerm の公式仕様であり、未対応端末でも安全に破棄される。ただし Base64 コストと 2 重通知の同期問題が残る。
  - **案C の特性**:
    - `OSC 133;P` は FinalTerm / WezTerm の公式仕様。未対応端末・旧 Kegon ともに無視され安全。
  - **案D の特性**:
    - `133;D;kegon.ps=success;...` は空スロット `;;` を含まず自己判別キーを持つが、**外部端末での取り扱いは未検証である**。
    - 特に C 言語標準の `atoi("kegon.ps=success")` は変換不能時に `0` を返却する仕様であるため、引数を数値検証なしに `atoi` に渡すナイーブな外部端末実装が存在する場合、**「終了コードなし」が「終了コード 0 (成功)」と誤認されるリスクが依然として存在する**。英字トークンであることだけをもって外部端末互換性を保証することはできない。

---

## 4. 方式比較と採否判定

| 評価軸 | 案A: OSC 133 D 引数 | 案B: 独立 SetUserVar | 案C: OSC 133 P Property | 案D: 構造化 D 判別子 (第一候補) |
|---|---|---|---|---|
| **境界同期性** | **最優秀** (D と完全同期) | **劣る** (2 通知のバッファ・非同期相関) | **劣る** (2 通知のバッファ・非同期相関) | **最優秀** (D と完全同期) |
| **プロトコル簡潔性** | **優秀** (単一 OSC) | **劣る** (Base64 エンコード必須) | **良好** (2 つの OSC) | **最優秀** (単一 OSC、Base64 不要) |
| **旧 Kegon 互換性** | **完全互換** (`Unknown` 縮退) | **完全互換** (1337 破棄) | **完全互換** (133;P 破棄) | **完全互換** (`Unknown` 縮退) |
| **他端末相互運用性** | **注意** (空スロット `;;` の `atoi` 0 誤読リスク) | **良好** (iTerm2/WezTerm 公式仕様) | **優秀** (FinalTerm 仕様準拠) | **未検証・要精査** (`atoi` の 0 誤変換リスク残存) |
| **シェル内負荷** | **13〜32 μs** (最速) | **27〜74 μs** (重い) | **15〜34 μs** (高速) | **14〜34 μs** (最速) |
| **異常系耐性** | **最優秀** (アトミック到着) | **普通** (欠落・重複・逆順管理が必要) | **普通** (欠落・重複・逆順管理が必要) | **最優秀** (アトミック到着) |

### 判定: **案D を受信基盤の第一候補として進めつつ、外部端末互換性を精査（PASS with notes）**

#### 選定理由と留意事項:
1. **アトミック性の確保**: コマンド終了通知 `D` とシェル報告成否 `kegon.ps` が 1 つの OSC メッセージで同時に届くため、別通知方式（案B・案C）のような「メタデータが先に来て D が来ない」「D が先に来てメタデータが遅れる」「メタデータの重複」といった複雑な非同期待ち・タイムアウト状態機械が一切不要。
2. **旧 Kegon パーサーとの 100% 後方互換性**: 現行 Kegon（`src/terminal/osc.rs`）は `D` の次のトークン `"kegon.ps=..."` を `i32` パース失敗として `None`（`CommandFinished(None)`）に正しく縮退し、既存の `CommandResult::Unknown` として破棄・処理される。
3. **シェル実行オーバーヘッド最小**: Base64 変換を行わず、PowerShell 7 で 34 μs、Windows PowerShell 5.1 で 14 μs と極めて高速。
4. **外部端末との互換性に関する留意事項（未検証）**:
   - 案D は英字トークンであっても `atoi` が 0 を返す外部端末実装での誤認リスクが残る。
   - したがって、**Phase 1 Slice 1 においては Rust 受信基盤として案D の構文解釈を先行実装するが、本番フックによる案D 送信はまだ確定させない**。

#### 案D と案C の最終選定に必要な追加検証事項:
1. 主要外部端末（iTerm2, WezTerm, VS Code, Windows Terminal）の OSC 133 D パーサー実装のソースコード調査（`atoi` 直結か、正規表現・数値バリデーション付きか）。
2. `133;D;kegon.ps=...` を受信させた場合の外部端末の実機挙動（終了ステータスマークが成功 0 に化けないかの確認）。
3. 外部端末の挙動に問題がある場合のフォールバックとして、FinalTerm 仕様準拠の `133;P`（案C）との切り替え可能性の保持。

---

## 5. 提案する具体的なメッセージ構文

### 5.1. コマンド開始時 (`OSC 133;C`)
```text
ESC ] 133 ; C ; kegon.v=1 ; kegon.sid=<session_id> ; kegon.seq=<seq> BEL
```
- `<session_id>`: PowerShell プロセス初期化時に生成された 16 桁の 16 進英数字（例: `A1B2C3D4E5F67890`）。
- `<seq>`: 非空コマンド実行ごとに 1 から始まる単調増加整数（`1, 2, 3, ...`）。
- 終端子: `BEL` (`0x07`) または `ST` (`ESC \`)。

### 5.2. コマンド終了時 (`OSC 133;D`)
#### ケース 1: 数値終了コードがない場合（PowerShell 実行の標準）
```text
ESC ] 133 ; D ; kegon.ps=<status> ; kegon.v=1 ; kegon.sid=<session_id> ; kegon.seq=<seq> BEL
```
- `<status>`: `success` または `failure`。不確かな場合は `unknown`。

#### ケース 2: 数値終了コードが存在する場合（ネイティブコマンドや将来の Bash 拡張）
```text
ESC ] 133 ; D ; <exit_code> ; kegon.ps=<status> ; kegon.v=1 ; kegon.sid=<session_id> ; kegon.seq=<seq> BEL
```
例: `ESC ] 133 ; D ; 42 ; kegon.ps=failure ; kegon.v=1 ; kegon.sid=A1B2 ; kegon.seq=1 BEL`

---

## 6. 相関および状態管理モデル

```mermaid
stateDiagram-v2
    [*] --> Idle: Tab/Session Created
    Idle --> Active: Receive 133;C (sid, seq)
    Active --> Idle: Receive 133;D (sid, seq, status) [DObserved]
    Active --> Idle: Receive 133;A/B [MissingD -> status=Unknown]
    Active --> Idle: Receive 133;D (mismatched sid/seq) [Desynchronized -> status=Unknown]
    Active --> [*]: Shell/Child Process Exited
```

### 相関ルール:
1. **Tab 分離**: 各タブ（`TabId`）は自身の PTY ストリームからのみイベントを受信し、他タブと状態を共有しない。
2. **単一 Active コマンド**: タブごとに同時に保持する active command は最大 1 件。
3. **厳格な一致**:
   - `active_command == Some((active_sid, active_seq))` と、受信した D の `(m.session_id, m.seq)` が完全一致する場合のみ、`m.shell_status` を採用。
   - `sid` 不一致（子シェル起動、フック再注入）または `seq` 不一致（順序乱れ、取りこぼし）の場合は即座に `Desynchronized` とし、`shell_status = Unknown` に縮退。
4. **終端ガード**:
   - D を受信した時点でコマンド状態を確定し、`active_command` をクリア（`None`）。
   - D の後に遅着した重複 D や通知は破棄。
   - D なしで次プロンプト `133;A` や `133;B` が届いた場合、`MissingD` として `shell_status = Unknown` で確定。

---

## 7. エラー・未知メッセージ時の縮退動作契約

| 状況 | `shell_status` | `osc133_exit` | `completion` |
|---|---|---|---|
| 正常終了（cmdlet 成功） | `ReportedSuccess` | `None` | `DObserved` |
| 正常終了（cmdlet 失敗） | `ReportedFailure` | `None` | `DObserved` |
| ネイティブ exit 42 後に失敗 | `ReportedFailure` | `Some(42)` | `DObserved` |
| ネイティブ exit 0 だがリダイレクト失敗 | `ReportedFailure` | `Some(0)` | `DObserved` |
| ユーザーが Ctrl+C で中断 | `Unknown` | `None` | `DObserved` または `MissingD` |
| 未知プロトコルバージョン (`v=99`) | `Unknown` | D 内の数値 (あれば) | `DObserved` |
| ペイロード破損・キー欠落 | `Unknown` | D 内の数値 (あれば) | `DObserved` |
| `seq` / `sid` 不一致 | `Unknown` | `None` | `Desynchronized` |
| プロンプトが先に来た (D 欠落) | `Unknown` | `None` | `MissingD` |
| 8KB 超過ペイロード | `Unknown` | `None` | `MissingD` (破棄) |

---

## 8. Rust 側の実装変更見積もり

製品実装（Phase 1）で必要な Rust 側の変更は局所的かつ安全です。

1. **`src/terminal/osc.rs`**:
   - `parse_osc133`:
     - 既存の `Osc133Marker` にメタデータ付きバリアントを追加、または `parse_osc133_v2` を導入。
     - トークンが整数か `kegon.*` かを判別。
     - 変更規模: 約 60 行。既存 unit tests 8 件はすべてそのまま維持・互換。
2. **`src/terminal/integration.rs`**:
   - 新規型定義:
     - `ShellReportedStatus { ReportedSuccess, ReportedFailure, Unknown }`
     - `CompletionReason { DObserved, MissingD, ShellExited, Desynchronized }`
     - `CommandExecutionResult { shell_status, osc133_exit, native_exit, completion }`
   - `CommandTracker` に `(session_id, seq)` の相関ロジックを追加。
   - 変更規模: 約 120 行。
3. **`src/terminal/session.rs` / `registry.rs`**:
   - 完了イベントの伝播シグネチャを `CommandExecutionResult` に更新。
   - 変更規模: 約 40 行。

---

## 9. Phase 1 以降の実装 Slice 案

Phase 0B の採否承認後、以下の段階的スライスで安全に main 統合へ進めることを提案します。

| Slice | 名称 | 対象スコープ | 完了条件 |
|---|---|---|---|
| **Slice 1** | **Rust コア結果型・相関状態機械** | `src/terminal/integration.rs`, `osc.rs`, `session.rs` | 独立結果型・Tab 別相関・終端ガード・10 項目のユニットテスト PASS。本番フックは未変更のまま後方互換を維持。 |
| **Slice 2** | **PowerShell フック実装・実機統合** | `assets/shell-integration/powershell.ps1`, 統合テスト | 案D の構文出力、Phase 0A の非干渉維持、PS7 実機 ConPTY 成否確認、WinPS 5.1 縮退確認。 |
| **Slice 3** | **UI 表示・i18n・Dopamine 型契約** | `app.rs`, `locales/*.ftl`, `docs/` | Tab Tooltip に「PowerShell報告成否」と「終了コード（OSC報告）」を分離表示。#62 Dopamine Mode 向けの型契約を整備。 |
| **Slice 4** | **最終回帰監査・PR 準備** | 全体 | 全テスト・静的解析（fmt, clippy, check）、PO 手動 GUI 確認、PR 作成。 |

---

## 10. 保全状況の確認

- **対象作業 worktree**: `C:\works\kegon-66-shell-status`
- **ブランチ**: `feature/66-powershell-reported-status`
- **HEAD**: `aa354c9963ab4ac610e3a33f5e185f9dda524a9b` (Clean、未コミット変更なし)
- **保全対象**:
  - 元 worktree `C:\works\kegon`: Clean、未接触。
  - `#62` worktree `C:\works\kegon-62-dopamine` (`feature/62-dopamine-mode-prototype`): Clean、未接触。
  - `#63` worktree `C:\works\kegon-63-shell-integration` (`feature/63-shell-integration-main`): Clean、未接触。
  - `stash@{0}`: 保持。
  - Alacritty Fork リビジョン: `758174acb2bf184cc55f177a56b8cf3292967f9f` 保持。
- **制約遵守**: 製品コード未変更、commit/push/PR/merge なし、GUI 自動キー送信なし。
